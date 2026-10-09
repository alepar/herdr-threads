//! Positive Claude Bash rendering: finite allow forms for ordinary commands, ask forms for
//! the immediate human namespace and for escalating commands.
use super::{PermissionInputs, arrangements};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClaudePermissionRules {
    pub allow: Vec<String>,
    pub ask: Vec<String>,
    /// Unsupported spellings and the declared conservative matcher limitations.
    pub diagnostics: Vec<String>,
}

/// Render only unquoted, literal single-shell-token spellings. Spaces are withheld
/// too: neither quoting normalization nor literal glob escaping has native evidence.
/// Trailing arguments (including output flags) are covered by a final wildcard;
/// their actor semantics remain enforced by the CLI parser, not this text model.
pub fn render(inputs: &PermissionInputs) -> ClaudePermissionRules {
    let catalog = inputs.catalog();
    let mut diagnostics = vec![
        "Claude coverage is a conservative text-matcher contract, not native classifier or live model evidence; compounds require separate constituent decisions.".into(),
        "Only catalogued literal leading global arrangements and unquoted executable spellings are declared covered; other global/quoting forms may prompt. Trailing arguments include --human output formatting; legacy operator/person options are refused by the CLI grammar, not filtered from arbitrary body text.".into(),
    ];
    diagnostics.extend(
        catalog
            .omissions
            .iter()
            .map(|(prefix, reason)| format!("{}: {reason}", prefix.join(" "))),
    );
    diagnostics.extend(catalog.families.iter().filter(|family| !family.human_options.is_empty()).map(|family| {
        format!("{} with {} may match a positive text prefix but is refused by the root CLI grammar; use immediate human for person/operator actions.", family.prefix.join(" "), family.human_options.join(" / "))
    }));
    for (label, path) in [
        ("state directory", &inputs.routing().state_directory),
        ("host endpoint", &inputs.routing().host_endpoint),
    ] {
        if let Some(path) = path {
            let value = path.to_str().expect("validated UTF-8 routing");
            if !representable(value) {
                diagnostics.push(format!("Claude cannot represent {label} {value} as a literal unquoted token; those pinned forms may prompt. Safe bare coverage remains."));
            }
        }
    }
    let routing = inputs.routing_tokens(representable);
    let mut allow = BTreeSet::new();
    let mut ask = BTreeSet::new();
    for executable in inputs.executables() {
        let spelling = executable.spelling.as_str();
        if !representable(spelling) {
            diagnostics.push(format!("Claude cannot represent executable {spelling} as a literal unquoted token; that spelling may prompt. Safe bare coverage remains."));
            continue;
        }
        add_prefix(&mut ask, &[spelling, "human"]);
        for family in catalog.families {
            for prefix in arrangements(spelling, &routing, family.prefix) {
                add_prefix(&mut allow, &prefix);
            }
        }
        // The CLI accepts an agent's escalating command only as its exact leading words, and
        // native ask rules win over allow rules, including a broader family such as `doctor`.
        for command in catalog.escalating {
            let mut prefix = vec![spelling];
            prefix.extend(command.iter().copied());
            add_prefix(&mut ask, &prefix);
        }
    }
    ClaudePermissionRules {
        allow: allow.into_iter().collect(),
        ask: ask.into_iter().collect(),
        diagnostics,
    }
}

fn representable(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"/_-.:@%+,=".contains(&b))
}

fn add_prefix(rules: &mut BTreeSet<String>, tokens: &[&str]) {
    let prefix = tokens.join(" ");
    rules.insert(format!("Bash({prefix})"));
    rules.insert(format!("Bash({prefix} *)"));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        cli::commands::{PermissionCliInputs, parse_argv},
        harness::{
            claude::bash_rule_covers,
            permissions::{PinnedRouting, VerifiedExecutableInventory},
        },
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
        fn new(name: &str, routing: PinnedRouting) -> Self {
            let root = std::env::temp_dir()
                .canonicalize()
                .unwrap()
                .join(format!("ht-permission-claude-{}", uuid::Uuid::new_v4()));
            fs::create_dir(&root).unwrap();
            let binary = root.join(name);
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
    fn covers(rules: &[String], command: &str) -> bool {
        rules.iter().any(|r| bash_rule_covers(r, command))
    }

    // Catches missing ordinary families, executable spellings or finite global forms.
    #[test]
    fn permission_claude_ordinary_forms() {
        let f = Fixture::new(
            "binary",
            PinnedRouting {
                state_directory: Some("/fixture/state".into()),
                host_endpoint: Some("/fixture/host.sock".into()),
            },
        );
        let rules = render(&f.inputs);
        eprintln!(
            "full fixture: allow={} ask={} total-rule-bytes={} max-rule-bytes={} compact-permissions-json-bytes={}",
            rules.allow.len(),
            rules.ask.len(),
            rules
                .allow
                .iter()
                .chain(&rules.ask)
                .map(String::len)
                .sum::<usize>(),
            rules
                .allow
                .iter()
                .chain(&rules.ask)
                .map(String::len)
                .max()
                .unwrap(),
            serde_json::to_vec(
                &serde_json::json!({"permissions": {"allow": rules.allow, "ask": rules.ask}})
            )
            .unwrap()
            .len()
        );
        for e in f.inputs.executables() {
            for tail in [
                "--help",
                "--version",
                "inbox --machine",
                "send THREAD --body 'human --operator'",
                "ack MESSAGE",
                "accept THREAD",
                "accept-required THREAD",
                "reject THREAD",
                "leave THREAD",
                "invite THREAD --pane P",
                "--human inbox",
                "inbox --human",
                "--state-dir /fixture/state --host-endpoint /fixture/host.sock --human inbox",
                "--host-endpoint /fixture/host.sock --state-dir /fixture/state send THREAD --body 'hello' --human",
                "--state-dir /fixture/state inbox",
                "--host-endpoint /fixture/host.sock inbox",
                "--json inbox",
                "--machine inbox",
                "inbox --json",
                "inbox --machine",
                "--state-dir /fixture/state --machine inbox",
                "--json --host-endpoint /fixture/host.sock inbox",
                "--human --state-dir /fixture/state inbox",
            ] {
                let command = format!("{} {tail}", e.spelling);
                assert!(
                    covers(&rules.allow, &command),
                    "ordinary form missing: {command}"
                );
            }
            assert!(!covers(&rules.allow, &format!("{}x inbox", e.spelling)));
        }
        for command in [
            "ht --state-dir /foreign inbox",
            "ht --state-dir=/fixture/state inbox",
            "ht --human --machine inbox",
            "'ht' inbox",
        ] {
            assert!(
                !covers(&rules.allow, command),
                "unsupported form covered: {command}"
            );
        }
    }

    // Catches broad allowances and confirms legacy flags are refused by real grammar.
    #[test]
    fn permission_claude_human_and_operator_exclusion() {
        let f = Fixture::new("binary", PinnedRouting::default());
        let rules = render(&f.inputs);
        for e in f.inputs.executables() {
            for tail in [
                "human",
                "human inbox",
                "human --state-dir /fixture/state invite THREAD --operator",
                "human retry REF",
            ] {
                let command = format!("{} {tail}", e.spelling);
                assert!(!covers(&rules.allow, &command), "human allowed: {command}");
                assert!(
                    covers(&rules.ask, &command),
                    "human prompt missing: {command}"
                );
            }
        }
        for args in [
            vec!["ht", "invite", "THREAD", "--pane", "P", "--operator"],
            vec![
                "ht",
                "seat",
                "resolve",
                "--pane",
                "P",
                "--new-seat",
                "--operator",
            ],
            vec!["ht", "me", "init"],
            vec!["ht", "seat", "retire", "SEAT", "--operator"],
            vec!["ht", "service", "disconnect"],
            vec!["ht", "--state-dir", "/fixture/state", "human", "inbox"],
        ] {
            assert!(
                parse_argv(args.clone()).is_err(),
                "legacy grammar accepted: {args:?}"
            );
        }
        assert!(
            parse_argv([
                "ht",
                "send",
                "THREAD",
                "--body",
                "human --operator",
                "--human"
            ])
            .is_ok()
        );
        assert!(!covers(&rules.allow, "ht me init"));
        assert!(!covers(&rules.allow, "ht seat retire SEAT --operator"));
    }

    // Catches widening text coverage or falsely claiming shell/classifier evidence.
    #[test]
    fn permission_claude_compounds_and_quote_limits() {
        let f = Fixture::new("binary", PinnedRouting::default());
        let rules = render(&f.inputs);
        for command in [
            "ht send THREAD --body 'human; ht human inbox'",
            "ht send THREAD --body \"human --operator\"",
            "ht send THREAD --body '$(ht human inbox)'",
        ] {
            assert!(
                covers(&rules.allow, command),
                "quoted data missing: {command}"
            );
        }
        for command in [
            "ht inbox; ht human inbox",
            "ht inbox && ht send THREAD --body hi",
            "ht inbox | cat",
            "ht send THREAD --body \"$(ht human inbox)\"",
            "ht send THREAD --body 'unfinished",
        ] {
            assert!(
                !covers(&rules.allow, command),
                "compound or ambiguous form covered: {command}"
            );
        }
        assert!(covers(&rules.allow, "ht inbox")); // Constituent only, not live decomposition.
        assert!(
            rules
                .diagnostics
                .iter()
                .any(|d| d.contains("conservative") && d.contains("native"))
        );
        assert!(
            rules
                .diagnostics
                .iter()
                .any(|d| d.contains("global") && d.contains("prompt"))
        );
    }

    // Catches unsafe interpolation of an owned spelling or pinned value.
    #[test]
    fn permission_claude_unrepresentable_paths_keep_bare_coverage() {
        for name in [
            "binary space",
            "binary'apostrophe",
            "binary\"quote",
            "binary*glob",
            "binary?glob",
            "binary[glob]",
            "binary\\slash",
            "binary$var",
            "binary;cmd",
            "binary`cmd`",
            "binary(paren)",
        ] {
            let f = Fixture::new(
                name,
                PinnedRouting {
                    state_directory: Some("/fixture/state space".into()),
                    host_endpoint: Some("/fixture/host*glob".into()),
                },
            );
            let rules = render(&f.inputs);
            assert!(covers(&rules.allow, "ht inbox"));
            let absolute = f.root.join(name).display().to_string();
            assert!(
                !rules
                    .allow
                    .iter()
                    .chain(&rules.ask)
                    .any(|r| r.contains(&absolute)),
                "unsupported spelling emitted: {name}"
            );
            assert!(
                rules.diagnostics.iter().any(|d| d.contains(&absolute)),
                "missing spelling diagnosis: {name}"
            );
            assert!(
                rules
                    .diagnostics
                    .iter()
                    .any(|d| d.contains("/fixture/state space"))
            );
            assert!(
                rules
                    .diagnostics
                    .iter()
                    .any(|d| d.contains("/fixture/host*glob"))
            );
            assert!(!covers(
                &rules.allow,
                "ht --state-dir '/fixture/state space' inbox"
            ));
            assert!(!covers(
                &rules.allow,
                "ht --host-endpoint /fixture/hostZZglob inbox"
            ));
        }
    }

    // An agent must not grant itself permissions: each escalating command asks by its leading
    // words, even where a broader ordinary family such as `doctor` allows.
    #[test]
    fn permission_claude_escalating_commands_ask() {
        let f = Fixture::new(
            "binary",
            PinnedRouting {
                state_directory: Some("/fixture/state".into()),
                host_endpoint: None,
            },
        );
        let rules = render(&f.inputs);
        for command in [
            "herdr-threads setup claude --with-permissions",
            "ht unsetup codex --json",
            "ht doctor fix --state-dir /fixture/state",
            "herdr-threads internal installer-integrations --confirm-missing",
        ] {
            assert!(covers(&rules.ask, command), "{command}");
        }
        // The grammar refuses these for an agent; natively they are not allowed either.
        for command in [
            "herdr-threads --json setup",
            "ht --state-dir /fixture/state unsetup claude",
        ] {
            assert!(!covers(&rules.allow, command), "{command}");
        }
        assert!(covers(&rules.allow, "herdr-threads doctor fix"));
        assert!(covers(&rules.allow, "herdr-threads setup-status claude"));
        assert!(!covers(&rules.ask, "herdr-threads setup-status claude"));
        assert!(!covers(&rules.ask, "herdr-threads doctor"));
        assert!(!covers(&rules.allow, "herdr-threads setup claude"));
    }
}
