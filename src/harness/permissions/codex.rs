//! Exact argv-prefix rules for an independently owned Codex rules file.
//!
//! Native strictest precedence preserves external forbidden/prompt policy. The
//! executable allow covers launch too; CLI grammar and semantic actor checks,
//! rather than arbitrary argument predicates, enforce the immediate human route.
//! Escalating commands prompt by their leading words, which the CLI requires an agent
//! to write first.
//! These rules do not establish shell decomposition or live classifier acceptance.
use super::PermissionInputs;

/// Exact Starlark string literals: JSON double-quoted UTF-8 strings share the
/// required quote/backslash escapes here (validated inputs contain no controls).
/// Spaces, apostrophes and shell metacharacters remain literal argv data.
pub fn render(inputs: &PermissionInputs) -> String {
    let spellings: Vec<&str> = inputs
        .executables()
        .iter()
        .map(|e| e.spelling.as_str())
        .collect();
    let union = serde_json::to_string(&spellings).expect("string serialization");
    let mut text = format!(
        "# herdr-threads owned execpolicy v1; exact argv prefixes\nprefix_rule(pattern = [{union}], decision = \"allow\")\nprefix_rule(pattern = [{union}, \"human\"], decision = \"prompt\")\n"
    );
    // The CLI accepts an agent's escalating command only as its exact leading words.
    for command in inputs.catalog().escalating {
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
    use crate::{
        cli::commands::PermissionCliInputs,
        harness::permissions::{PinnedRouting, VerifiedExecutableInventory},
    };
    use sha2::{Digest, Sha256};
    use std::{
        fs,
        os::unix::fs::{PermissionsExt, symlink},
        path::PathBuf,
    };

    struct Fixture {
        root: PathBuf,
        inputs: PermissionInputs,
    }
    impl Fixture {
        fn new(routing: PinnedRouting) -> Self {
            let root = std::env::temp_dir()
                .canonicalize()
                .unwrap()
                .join(format!("ht-permission-codex-{}", uuid::Uuid::new_v4()));
            fs::create_dir(&root).unwrap();
            let binary = root.join("binary space'quote\"double\\slash*$;`(é)");
            fs::write(&binary, b"owned").unwrap();
            fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
            let links = [root.join("herdr-threads"), root.join("ht")];
            for link in &links {
                symlink(&binary, link).unwrap();
            }
            let inventory = VerifiedExecutableInventory::verify(
                &binary,
                &links,
                &format!("{:x}", Sha256::digest(b"owned")),
            )
            .unwrap();
            let cli = PermissionCliInputs {
                permission_installed_binary: Some(binary.display().to_string()),
                permission_link_path: Some(links[0].display().to_string()),
                permission_alias_path: Some(links[1].display().to_string()),
                ..Default::default()
            };
            let inputs = PermissionInputs::validate(&cli, &inventory, routing).unwrap();
            Self { root, inputs }
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.root).unwrap();
        }
    }

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
        let f = Fixture::new(PinnedRouting::default());
        let (union, rules) = decode(&render(&f.inputs));
        assert_eq!(
            union,
            f.inputs
                .executables()
                .iter()
                .map(|e| e.spelling.clone())
                .collect::<Vec<_>>()
        );
        assert_eq!(&union[..2], &["herdr-threads", "ht"]);
        assert!(union[2].ends_with("binary space'quote\"double\\slash*$;`(é)"));
        assert_eq!(rules[0], (vec![], "allow".to_owned()));
        assert_eq!(rules[1], (vec!["human".to_owned()], "prompt".to_owned()));
    }

    // An agent must not grant itself permissions: every escalating command prompts by its
    // leading words, while ordinary commands stay allowed.
    #[test]
    fn permission_codex_escalating_commands_prompt() {
        let f = Fixture::new(PinnedRouting::default());
        let (_, rules) = decode(&render(&f.inputs));
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
