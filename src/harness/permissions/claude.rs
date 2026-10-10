//! Claude Bash rules: allow each spelling outright; ask for the immediate `human` namespace and
//! for every escalating (self-granting) command. Claude applies ask before allow.
use super::SPELLINGS;
use crate::cli::commands::ordinary_catalog;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClaudePermissionRules {
    pub allow: Vec<String>,
    pub ask: Vec<String>,
}

/// `Bash(P)` matches the bare command, `Bash(P *)` the command with arguments.
fn prefix(rules: &mut Vec<String>, words: &[&str]) {
    let prefix = words.join(" ");
    rules.push(format!("Bash({prefix})"));
    rules.push(format!("Bash({prefix} *)"));
}

pub fn render() -> ClaudePermissionRules {
    let mut rules = ClaudePermissionRules::default();
    for spelling in SPELLINGS {
        prefix(&mut rules.allow, &[spelling]);
        prefix(&mut rules.ask, &[spelling, "human"]);
        // The CLI accepts an agent's escalating command only as its exact leading words.
        for command in ordinary_catalog().escalating {
            let mut words = vec![spelling];
            words.extend(command.iter().copied());
            prefix(&mut rules.ask, &words);
        }
    }
    rules
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::claude::bash_rule_covers;

    fn decision(rules: &ClaudePermissionRules, command: &str) -> &'static str {
        if rules.ask.iter().any(|r| bash_rule_covers(r, command)) {
            "ask"
        } else if rules.allow.iter().any(|r| bash_rule_covers(r, command)) {
            "allow"
        } else {
            "none"
        }
    }

    // Ordinary commands of both spellings run, with any arguments or leading routing; the
    // human namespace and every self-granting command ask, bare or with arguments.
    #[test]
    fn permission_claude_allows_spellings_and_asks_human_and_escalating() {
        let rules = render();
        assert_eq!(rules.allow.len(), 4);
        for spelling in SPELLINGS {
            for (args, expected) in [
                ("", "allow"),
                (" inbox", "allow"),
                (" --state-dir /s --json send t --body x", "allow"),
                (" doctor --debug", "allow"),
                (" human", "ask"),
                (" human me init", "ask"),
                (" setup claude --json", "ask"),
                (" unsetup", "ask"),
                (" doctor fix", "ask"),
                (" internal installer-integrations x", "ask"),
            ] {
                let command = format!("{spelling}{args}");
                assert_eq!(decision(&rules, &command), expected, "{command}");
            }
        }
        assert_eq!(decision(&rules, "herdr-threadsx inbox"), "none");
        assert_eq!(decision(&rules, "/usr/local/bin/ht inbox"), "none");
    }
}
