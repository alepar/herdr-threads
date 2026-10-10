//! Exact argv-prefix rules for an independently owned Codex rules file.
//!
//! Native strictest precedence preserves external forbidden/prompt policy. The
//! executable allow covers launch too; CLI grammar and semantic actor checks,
//! rather than arbitrary argument predicates, enforce the immediate human route.
//! Escalating commands prompt by their leading words, which the CLI requires an agent
//! to write first.
//! These rules do not establish shell decomposition or live classifier acceptance.
use super::SPELLINGS;

/// Exact Starlark string literals: JSON double-quoted strings share the escapes needed here.
pub fn render() -> String {
    let union = serde_json::to_string(&SPELLINGS).expect("string serialization");
    let mut text = format!(
        "# herdr-threads owned execpolicy v1; exact argv prefixes\nprefix_rule(pattern = [{union}], decision = \"allow\")\nprefix_rule(pattern = [{union}, \"human\"], decision = \"prompt\")\n"
    );
    // The CLI accepts an agent's escalating command only as its exact leading words.
    for command in crate::cli::commands::ordinary_catalog().escalating {
        let rest: String = command
            .iter()
            .map(|token| format!(", {}", serde_json::to_string(token).expect("string")))
            .collect();
        text.push_str(&format!(
            "prefix_rule(pattern = [{union}{rest}], decision = \"prompt\")\n"
        ));
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Decoded rules as (exact argv tokens after the union, decision), plus the union.
    fn decode(text: &str) -> (Vec<String>, Vec<(Vec<String>, String)>) {
        let mut union = None;
        let mut rules = Vec::new();
        for line in text.lines().filter(|l| !l.starts_with('#')) {
            let body = line.strip_prefix("prefix_rule(pattern = [").unwrap();
            let (pattern, decision) = body.split_once("], decision = ").unwrap();
            let tokens: Vec<serde_json::Value> =
                serde_json::from_str(&format!("[{pattern}]")).unwrap();
            let first: Vec<String> = serde_json::from_value(tokens[0].clone()).unwrap();
            assert_eq!(*union.get_or_insert(first.clone()), first);
            let rest = tokens[1..]
                .iter()
                .map(|t| t.as_str().unwrap().to_owned())
                .collect();
            let decision: String =
                serde_json::from_str(decision.strip_suffix(')').unwrap()).unwrap();
            rules.push((rest, decision));
        }
        (union.unwrap(), rules)
    }

    // Missing union/prompt or lossy escaping must fail exact decoded argv token assertions.
    #[test]
    fn permission_codex_exact_union_and_human_prompt() {
        let (union, rules) = decode(&render());
        assert_eq!(union, SPELLINGS);
        assert_eq!(rules[0], (vec![], "allow".to_owned()));
        assert_eq!(rules[1], (vec!["human".to_owned()], "prompt".to_owned()));
    }

    // An agent must not grant itself permissions: every escalating command prompts by its
    // leading words, while ordinary commands stay allowed.
    #[test]
    fn permission_codex_escalating_commands_prompt() {
        let (_, rules) = decode(&render());
        let prompts: Vec<Vec<String>> = rules
            .iter()
            .filter(|(_, d)| d == "prompt")
            .map(|(t, _)| t.clone())
            .collect();
        let expected: Vec<Vec<String>> = [
            &["human"][..],
            &["setup"],
            &["unsetup"],
            &["doctor", "fix"],
            &["internal", "installer-integrations"],
        ]
        .iter()
        .map(|words| words.iter().map(|w| (*w).to_owned()).collect())
        .collect();
        assert_eq!(prompts, expected);
    }
}
