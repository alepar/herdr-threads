use super::*;

fn run(words: &[&str]) -> String {
    let mut out = Vec::new();
    crate::cli::run_in_pane(words.iter().copied(), None, &mut out).unwrap();
    String::from_utf8(out).unwrap()
}

fn operator_recipe(recipe: &str, disconnect: bool) {
    use crate::cli::{
        actor_route::InvocationActor,
        commands::{CliAction, parse_argv},
    };
    use crate::protocol::commands::Command;
    let boot = "00000000-0000-4000-8000-000000000001";
    let mut words = shlex::split(recipe).unwrap();
    if words[0] != "herdr-threads" {
        words.insert(0, "herdr-threads".into());
    }
    for word in &mut words {
        match word.as_str() {
            "BOOT" => *word = boot.into(),
            "GENERATION" => *word = "7".into(),
            _ => {}
        }
    }
    // Feed the shipped command directly to the production actor parser before
    // adding any context. A missing immediate namespace must fail this test.
    let parsed = parse_argv(words.clone())
        .unwrap_or_else(|error| panic!("compiled recipe refused: {recipe}: {error:?}"));
    assert_eq!(parsed.actor, InvocationActor::Human);
    let mut rooted = words.clone();
    rooted.remove(1);
    assert!(
        parse_argv(rooted.clone()).is_err(),
        "root Agent must refuse person/operator action"
    );
    rooted.insert(1, "--human".into());
    assert!(
        parse_argv(rooted).is_err(),
        "output-only --human grants no actor authority"
    );
    words.splice(
        2..2,
        [
            "--state-dir".into(),
            "/private/tmp/state's $` dir".into(),
            "--host-endpoint".into(),
            "/private/tmp/host' $.sock".into(),
            "--human".into(),
        ],
    );
    let routed = parse_argv(words).unwrap();
    assert_eq!(routed.actor, InvocationActor::Human);
    assert_eq!(
        routed.output.context.state_dir.as_deref(),
        Some("/private/tmp/state's $` dir")
    );
    assert_eq!(
        routed.output.context.host.as_deref(),
        Some("/private/tmp/host' $.sock")
    );
    if disconnect {
        assert!(
            matches!(routed.action, CliAction::Wire(Command::ServiceDisconnect(request)) if request.expected_boot == boot && request.expected_generation == 7)
        );
    } else {
        assert!(matches!(routed.action, CliAction::MeInit { .. }));
    }
}

// Catches compiled recovery instructions that the enforced actor parser refuses.
// These tests parse administrative recipes; they never execute those actions.
#[test]
fn final_wave_compiled_guide_service_recovery_is_lawful() {
    let guide = run(&["herdr-threads", "skill"]);
    let paragraph = guide
        .lines()
        .find(|line| line.starts_with("For service connection recovery,"))
        .unwrap();
    let recipe = paragraph
        .split('`')
        .find(|part| part.contains("service disconnect"))
        .unwrap();
    operator_recipe(recipe, true);
    let inspect = paragraph
        .split('`')
        .find(|part| part.contains("service inspect"))
        .unwrap();
    assert!(matches!(
        crate::cli::commands::parse_argv(shlex::split(inspect).unwrap())
            .unwrap()
            .action,
        crate::cli::commands::CliAction::Wire(crate::protocol::commands::Command::ServiceInspect)
    ));
}

#[test]
fn final_wave_compiled_guide_human_binding_is_lawful() {
    let guide = run(&["herdr-threads", "skill"]);
    let paragraph = guide
        .lines()
        .find(|line| line.starts_with("A human binding ("))
        .unwrap();
    let recipe = paragraph
        .split('`')
        .find(|part| part.contains("me init"))
        .unwrap();
    operator_recipe(recipe, false);
}

#[test]
fn final_wave_compiled_service_help_recovery_is_lawful() {
    let help = run(&["herdr-threads", "service", "--help"]);
    let recipe = help
        .lines()
        .find(|line| {
            line.trim_start().starts_with("herdr-threads") && line.contains("service disconnect")
        })
        .unwrap();
    operator_recipe(recipe.trim(), true);
}

// Kills: printing anything but the checked-in file, or `--skill` diverging
// from `skill`, or either needing a daemon/state context (no pane, no state
// directory here).
#[test]
fn skill_and_long_flag_print_the_checked_in_file() {
    let file = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/integrations/skill/SKILL.md"
    ))
    .unwrap();
    assert_eq!(SKILL_MD, file);
    assert_eq!(run(&["herdr-threads", "skill"]), file);
    assert_eq!(run(&["herdr-threads", "--skill"]), file);
}

// Kills: a skill file without the standard frontmatter, or one that drops the
// trust model, ACK meaning, subagent rule or the --help pointer.
#[test]
fn skill_file_is_a_well_formed_concise_guide() {
    let mut lines = SKILL_MD.lines();
    assert_eq!(lines.next(), Some("---"));
    assert_eq!(lines.next(), Some("name: herdr-threads"));
    assert!(lines.next().unwrap().starts_with("description: "));
    assert_eq!(lines.next(), Some("---"));
    assert!(SKILL_MD.lines().count() < 200);
    let summary_header = format!("## {}", crate::protocol::summary::SUMMARY_PROCEDURE_REF);
    for needle in [
        summary_header.as_str(),
        "summary job",
        "summary submit",
        "--relays-user",
        SUMMARY_PROMPT_VERSION,
        "Summary, one job, Summary",
        "never ACK",
        "cooperative, not enforced",
        "receipt only",
        "never** run `ack`",
        "accept-required THREAD --invitation ID --requirement ID --revision N",
        "pending-receipts",
        "send THREAD --body",
        "herdr-threads <command> --help",
        "Ready commands (run exactly as written, in this pane):",
    ] {
        assert!(SKILL_MD.contains(needle), "{needle}");
    }
}

#[test]
fn skill_describes_text_inbox_display_ack_and_read_only_subagent_path() {
    for required in [
        "default text `inbox`",
        "written and flushed",
        "fully displayed",
        "`inbox --machine`",
        "`--json`",
        "continuation",
    ] {
        assert!(SKILL_MD.contains(required), "missing {required}");
    }
}

// Kills: a detector that accepts any mention of the title, or one that misses
// the section header.
#[test]
fn summary_section_is_detected() {
    assert!(has_summary_procedure(SKILL_MD));
    let header = format!("## {}\n", crate::protocol::summary::SUMMARY_PROCEDURE_REF);
    assert!(SKILL_MD.contains(&header));
    let without = SKILL_MD.replace(&header, "");
    assert!(!has_summary_procedure(&without));
}

// Kills: dropping the AI pointer from the top-level help, or replacing the
// exit-status table with it.
#[test]
fn top_level_help_ends_with_the_ai_pointer() {
    let help = run(&["herdr-threads", "--help"]);
    assert!(help.contains("Exit status:"), "{help}");
    assert!(help.trim_end().ends_with(AI_HELP_FOOTER), "{help}");
    assert!(help.contains("skill, --skill"), "{help}");
}

// Kills: the human presentation (terminal default or `--human`) rewriting the
// skill guide, or `--machine` changing it; the markdown prints verbatim.
#[test]
fn skill_prints_verbatim_in_every_presentation() {
    for terminal in [true, false] {
        for words in [
            &["herdr-threads", "skill"][..],
            &["herdr-threads", "--human", "skill"][..],
            &["herdr-threads", "skill", "--machine"][..],
            &["herdr-threads", "--skill"][..],
        ] {
            let mut out = Vec::new();
            crate::cli::run_terminal(words.iter().copied(), &mut out, terminal).unwrap();
            assert_eq!(
                String::from_utf8(out).unwrap(),
                SKILL_MD,
                "{words:?} {terminal}"
            );
        }
    }
    crate::cli::output::set_stdout_is_terminal(false);
}

// Kills: the blanket "--json on any command" claim coming back (`skill`
// prints the guide whatever the flag says, and `--json` conflicts with
// --human/--machine), and the digest example sitting inside the block of
// ready commands the agent is told to run verbatim.
#[test]
fn skill_does_not_overclaim_json_and_keeps_the_digest_out_of_the_ready_block() {
    assert!(
        !SKILL_MD.contains("to any command for structured output"),
        "the blanket --json claim must stay gone"
    );
    assert!(SKILL_MD.contains("`--json` selects JSON output for commands that return a result"));
    let block = SKILL_MD
        .split("```")
        .find(|block| block.contains("Ready commands (run exactly as written"))
        .expect("the ready commands block");
    assert!(
        !block.contains("attention digest:"),
        "the digest line is not a ready command: {block}"
    );
    assert!(SKILL_MD.contains("`attention digest: invitations=1 [INV@THREAD]"));
}

// Kills: a compact renderer inventing a `--offset` continuation: the daemon
// always supplies `body_next_argv`, so the fallback was dead code.
#[test]
fn compact_continuations_never_build_an_offset_argv() {
    let source = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/protocol/output_compact.rs"
    ))
    .unwrap();
    assert!(
        !source.contains("\"--offset\""),
        "output_compact.rs must not build a --offset argv"
    );
}

#[test]
fn user_intent_guidance_schema2_and_lifetimes() {
    let printed = run(&["herdr-threads", "skill"]);
    assert_eq!(printed, SKILL_MD);
    assert_eq!(run(&["herdr-threads", "--skill"]), printed);
    let schema = format!(
        "\"submission_schema\":{}",
        crate::protocol::summary::SUBMISSION_SCHEMA
    );
    assert!(
        printed.contains(&schema),
        "worker must submit current schema"
    );
    assert!(printed.contains(SUMMARY_PROMPT_VERSION));
    let example = printed
        .split('`')
        .find(|part| part.starts_with("{\"submission_schema\":"))
        .expect("literal worker JSON example");
    let object: serde_json::Value = serde_json::from_str(example).unwrap();
    assert_eq!(
        object["submission_schema"],
        crate::protocol::summary::SUBMISSION_SCHEMA
    );
    assert_eq!(object["prompt_version"], SUMMARY_PROMPT_VERSION);
    for needle in [
        "rule_change?",
        "withdrawn",
        "replaced",
        "exact nonempty quote",
        "partial answers",
        "promises",
        "ACKs",
        "supplied live ledger",
        "earlier unstored",
        "current chunk",
        "no duplicate",
        "direct human",
        "independently actionable",
        "uncertain duration",
        "classification grants no permission",
        "no automatic resolution on send or inbox",
    ] {
        assert!(printed.contains(needle), "missing {needle}");
    }
    for example in [
        "herdr-threads send THREAD --relays-user --user-intent query --body \"What's our progress?\"",
        "herdr-threads send THREAD --relays-user --user-intent request --body \"Let's cut a build.\"",
        "herdr-threads send THREAD --relays-user --user-intent rule --body \"Always run tests before cutting a release.\"",
    ] {
        assert!(printed.contains(example), "missing {example}");
    }
}

#[test]
fn user_intent_guidance_preserves_codex_permissions_verbatim() {
    const PARAGRAPH: &str = "Codex: run `herdr-threads` / `ht` outside the sandbox through a CLI-only approved rule; otherwise request `sandbox_permissions=\"require_escalated\"` with justification and a CLI-only `prefix_rule`, never a shell rule. With approval `never` (including `exec`), use ordinary calls with a preapproved rule. Keep other commands sandboxed; report refused/unavailable permission, never bypass policy or enable networking.";
    assert!(SKILL_MD.lines().any(|line| line == PARAGRAPH));
    assert_eq!(
        CODEX_COMMAND_GUIDANCE,
        "Codex: run herdr-threads (ht) commands outside the sandbox through an approved CLI-only command rule. Without a rule, request sandbox_permissions=\"require_escalated\" with a short justification and CLI-only prefix_rule, never a shell rule. With approval never (including exec), use ordinary shell calls with a preapproved CLI rule; explicit escalation is unavailable. Keep other commands sandboxed. If approval is refused or unavailable, report it; never bypass policy or enable networking.\n"
    );
}

#[test]
fn user_intent_guidance_query_withdrawal() {
    let printed = run(&["herdr-threads", "skill"]);
    for needle in [
        "Query withdrawal/replacement must cite explicit ordinary human/relayed input",
        "a replacement question gets its own separately classified source",
        "`cite_seq` must be in the current chunk and strictly later than the target's source",
        "partial answers, promises, silence and ACKs never establish completion",
        "Uncertain evidence leaves the entry open",
        "Agent answers and completion reports can be evidence",
        "Closure is worker judgment; the daemon checks structure, allowed status and citation evidence",
    ] {
        assert!(printed.contains(needle), "missing {needle}");
    }
}

#[test]
fn daily_loop_uses_inbox_and_hook_notifications() {
    let daily = SKILL_MD
        .split_once("## Daily loop (top-level agent)")
        .unwrap()
        .1
        .split_once("## Human input")
        .unwrap()
        .0;
    let commands = daily
        .split_once("```bash\n")
        .unwrap()
        .1
        .split_once("```")
        .unwrap()
        .0;
    assert!(commands.contains("herdr-threads inbox"));
    assert!(commands.contains("herdr-threads send"));
    for verb in ["read", "body", "follow", "ack", "pending-receipts"] {
        assert!(
            !commands.contains(&format!("herdr-threads {verb}")),
            "routine command: {verb}"
        );
    }
    assert!(daily.contains("finish your native turn"));
    assert!(daily.contains("earlier context absent from inbox"));
    assert!(daily.contains("long inbox messages use inbox continuations"));
}
