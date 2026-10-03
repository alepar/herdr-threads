use super::*;

fn run(words: &[&str]) -> String {
    let mut out = Vec::new();
    crate::cli::run_in_pane(words.iter().copied(), None, &mut out).unwrap();
    String::from_utf8(out).unwrap()
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
    assert!(SKILL_MD.lines().count() < 150);
    for needle in [
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
