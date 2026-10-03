//! `herdr-threads skill` (alias `--skill`): print the embedded agent skill.
//!
//! The text is `integrations/skill/SKILL.md`, compiled in so the installed
//! binary always prints the guide that matches its own CLI. It never touches
//! the daemon, the state directory or the host.

/// The agent skill file, in standard skill format (frontmatter name and
/// description, then a concise guide that defers to `--help` for syntax).
pub const SKILL_MD: &str = include_str!("../../integrations/skill/SKILL.md");

/// Footer of the top-level `--help`: points an AI agent at the skill.
pub const AI_HELP_FOOTER: &str = "Are you an AI agent? Run `herdr-threads skill` for the agent guide (skip it if a herdr-threads skill is already in your context); then read `herdr-threads <command> --help` as needed.";

/// One line the SessionStart hook adds to its fixed instructions.
pub const HOOK_SKILL_HINT: &str =
    "Agent guide: run herdr-threads skill; command syntax: herdr-threads <command> --help.";

/// The `prompt_version` summary workers send; the skill's worker prompt names it.
pub const SUMMARY_PROMPT_VERSION: &str = "thread-summary-v1";

/// Whether `text` carries the thread-summary procedure section (a line equal
/// to `## ` plus the contract's `SUMMARY_PROCEDURE_REF`).
pub fn has_summary_procedure(text: &str) -> bool {
    let header = format!("## {}", crate::protocol::summary::SUMMARY_PROCEDURE_REF);
    text.lines().any(|line| line == header)
}

#[cfg(test)]
#[path = "../../tests/cli/skill.rs"]
mod tests;
