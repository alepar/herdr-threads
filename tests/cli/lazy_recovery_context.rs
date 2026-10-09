//! Execute settlement's printed retries without repairing their actor or route.
use crate::lazy_config_smoke::{Proxy, World};
use herdr_threads::{
    cli::journal::{IntentScope, Journal, SemanticMutation},
    daemon::ownership::read_existing_namespace,
    harness::context::ContextJournal,
    protocol::{authority::CallerClaim, ids::MessageId},
    test_support::spawn,
};
use sha2::{Digest, Sha256};
use std::{path::PathBuf, process::Output, sync::atomic::Ordering, time::Duration};

fn claim(w: &World, who: usize) -> CallerClaim {
    let paths = w.paths();
    let instance = read_existing_namespace(&paths).unwrap().unwrap();
    let contexts = ContextJournal::open(
        &paths
            .instance_dir
            .join("contexts")
            .join(format!("{:x}", Sha256::digest(w.seats[who].as_bytes()))),
        instance,
        &w.seats[who],
        Duration::from_secs(1),
    )
    .unwrap();
    herdr_threads::harness::bridge::caller_claim(&contexts.current().unwrap().unwrap()).unwrap()
}

fn execute(w: &World, ambient: Option<&World>, who: usize, argv: &[String]) -> Output {
    assert_eq!(argv[0], "herdr-threads");
    let mut command = spawn::command(env!("CARGO_BIN_EXE_herdr-threads"));
    command
        .args(&argv[1..])
        .env("HOME", w.root.join("home"))
        .env("CLAUDE_CONFIG_DIR", w.root.join("claude"))
        .env("CODEX_HOME", w.root.join("codex"))
        .env("HERDR_THREADS_OFFLINE", "1")
        .env("NO_COLOR", "1")
        .env("HERDR_PANE_ID", format!("w1:p{}", who + 1));
    if let Some(ambient) = ambient {
        let paths = ambient.paths();
        command
            .env(
                "HERDR_PLUGIN_STATE_DIR",
                paths.instance_dir.parent().unwrap().parent().unwrap(),
            )
            .env("HERDR_SOCKET_PATH", paths.locator);
    } else {
        command
            .env_remove("HERDR_PLUGIN_STATE_DIR")
            .env_remove("HERDR_SOCKET_PATH");
    }
    command.output().unwrap()
}

fn receipt(w: &World, message: &str, who: usize) -> String {
    w.db().query_row(
        "SELECT state FROM receipt_state WHERE message_id=?1 AND seat_id=?2 UNION SELECT state FROM receipts WHERE message_id=?1 AND seat_id=?2",
        [message, &w.seats[who]], |r| r.get(0),
    ).unwrap()
}

// Catches journal-local references selecting ambient B instead of the pinned A,
// dropped quoting/host context, actor replacement, or clearing independent proof.
//
// Each case runs two ambient controls (explicit A against a conflicting ambient
// B, and environment-only A). Each control is its own #[test] (its own process
// under nextest, so they run in parallel): World setup (a daemon plus ~15 CLI
// calls, twice) dominates the cost.
fn recovery_case(human: bool, second_record_failure: bool, remove_ambient: bool) {
    {
        let a = World::with_routing_names("state's $` dir", "h' $.sock");
        let b = World::new();
        let who = if human { 2 } else { 1 };
        let lazy_a = a.send("original passive body", &[]);
        let ordinary_a = a.send("original ordinary body", &["--require-ack", &a.seats[1]]);
        let lazy_b = b.send("unrelated passive body", &[]);
        let ordinary_b = b.send("unrelated ordinary body", &["--require-ack", &b.seats[1]]);
        let journal_a = Journal::open(a.paths().instance_dir.join("intents")).unwrap();
        if second_record_failure {
            std::fs::write(
                a.paths().instance_dir.join("intents/next-ordinal"),
                format!("{}\n", u64::MAX - 1),
            )
            .unwrap();
        }
        let proxy = Proxy::new(a.paths(), &a.root);
        if !second_record_failure {
            proxy.mode.store(3, Ordering::SeqCst);
        }
        // Explicit A wins over a conflicting ambient B. The other control starts
        // with environment-only A, which production must pin before settlement.
        let paths_a = a.paths();
        let state_a = paths_a.instance_dir.parent().unwrap().parent().unwrap();
        let mut inbox = vec!["herdr-threads".to_owned()];
        if human {
            inbox.push("human".into());
        }
        if !remove_ambient {
            inbox.extend([
                "--state-dir".into(),
                state_a.to_string_lossy().into_owned(),
                "--host-endpoint".into(),
                paths_a.locator.clone(),
            ]);
        }
        inbox.push("inbox".into());
        let initial = execute(&a, Some(if remove_ambient { &a } else { &b }), who, &inbox);
        assert!(
            !initial.status.success(),
            "settlement must fail in this fixture"
        );
        let error = String::from_utf8(initial.stderr).unwrap();
        let commands: Vec<_> = error
            .split("; retry ")
            .skip(1)
            .map(|s| shlex::split(s.split(';').next().unwrap().trim()).unwrap())
            .collect();
        assert_eq!(
            commands.len(),
            if human || second_record_failure { 1 } else { 2 },
            "{error}"
        );
        assert!(
            String::from_utf8(initial.stdout)
                .unwrap()
                .contains("original passive body")
        );
        let frozen_a = claim(&a, who);
        let frozen_b = claim(&b, who);
        let scope_a = IntentScope::Cooperative {
            instance: frozen_a.instance.clone(),
            seat: frozen_a.seat.clone(),
        };
        let scope_b = IntentScope::Cooperative {
            instance: frozen_b.instance.clone(),
            seat: frozen_b.seat.clone(),
        };
        journal_a
            .record_lazy_displayed_chunk(&frozen_a, &MessageId::new("unrelated-proof"), 0, 4, 4)
            .unwrap();
        let journal_b = Journal::open(b.paths().instance_dir.join("intents")).unwrap();
        let mut original_commands = vec![];
        for argv in &commands {
            let reference = journal_a
                .resolve_recovery_ref(argv.last().unwrap())
                .unwrap();
            let pending = journal_a.load(&reference).unwrap();
            assert_eq!(pending.header.scope, scope_a);
            assert_eq!(pending.semantic.frozen_claim(), Some(&frozen_a));
            let command = pending
                .semantic
                .to_command(reference.operation.clone(), None)
                .unwrap();
            let counterpart = match &command {
                herdr_threads::protocol::commands::Command::AckDisplayed(_) => {
                    SemanticMutation::AckDisplayed {
                        messages: vec![MessageId::new(&ordinary_b)],
                    }
                }
                herdr_threads::protocol::commands::Command::CompleteInboxDelivery(_) => {
                    SemanticMutation::CompleteInboxDelivery {
                        messages: vec![MessageId::new(&lazy_b)],
                    }
                }
                other => panic!("unexpected settlement {other:?}"),
            };
            original_commands.push(serde_json::to_value(command).unwrap());
            std::fs::write(
                b.paths().instance_dir.join("intents/next-ordinal"),
                format!("{}\n", reference.ordinal - 1),
            )
            .unwrap();
            let unrelated = journal_b
                .record(
                    scope_b.clone(),
                    SemanticMutation::freeze(counterpart, frozen_b.clone()).unwrap(),
                    1,
                )
                .unwrap();
            assert_eq!(unrelated.ordinal, reference.ordinal);
        }
        let b_intents = b.intents();
        let initial_submissions: Vec<_> = proxy
            .requests()
            .into_iter()
            .filter(|v| {
                matches!(
                    v["command"]["kind"].as_str(),
                    Some("ack_displayed" | "complete_inbox_delivery")
                )
            })
            .map(|v| v["command"].clone())
            .collect();
        if second_record_failure {
            assert!(
                initial_submissions.is_empty(),
                "record all intents before any submission"
            );
        } else {
            assert_eq!(
                initial_submissions, original_commands,
                "retained payload must match the original submitted commands"
            );
        }
        let requests_before = proxy.requests().len();
        proxy.mode.store(0, Ordering::SeqCst);
        let mut statuses = vec![];
        for argv in &commands {
            let output = execute(&a, (!remove_ambient).then_some(&b), who, argv);
            statuses.push((
                argv.clone(),
                output.status.code(),
                String::from_utf8(output.stderr).unwrap(),
            ));
        }
        let replayed: Vec<_> = proxy
            .requests()
            .into_iter()
            .skip(requests_before)
            .filter(|v| {
                matches!(
                    v["command"]["kind"].as_str(),
                    Some("ack_displayed" | "complete_inbox_delivery")
                )
            })
            .map(|v| v["command"].clone())
            .collect();
        let original_pending = commands
            .iter()
            .any(|argv| journal_a.resolve_recovery_ref(argv.last().unwrap()).is_ok());
        let effects_ok = statuses.iter().all(|(_, status, _)| *status == Some(0))
            && replayed == original_commands
            && !original_pending
            && b.intents() == b_intents
            && b.state(&lazy_b, who) == "pending"
            && receipt(&b, &ordinary_b, 1) == "pending"
            && a.state(&lazy_a, who)
                == if second_record_failure {
                    "pending"
                } else {
                    "displayed"
                }
            && (human || receipt(&a, &ordinary_a, who) == "acked");
        let proof_files: Vec<PathBuf> = std::fs::read_dir(paths_a.instance_dir.join("intents"))
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| {
                p.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("display-lazy-")
            })
            .collect();
        let effects_ok =
            effects_ok && proof_files.len() == if second_record_failure { 2 } else { 1 };
        let evidence = format!(
            "human={human} second_record_failure={second_record_failure} remove_ambient={remove_ambient}; emitted={commands:?}; statuses={statuses:?}; replayed={replayed:?}; original_pending={original_pending}; A_lazy={} B_lazy={} B_receipt={} B_journal_unchanged={}",
            a.state(&lazy_a, who),
            b.state(&lazy_b, who),
            receipt(&b, &ordinary_b, 1),
            b.intents() == b_intents
        );
        println!("{evidence}");
        assert!(
            effects_ok,
            "printed recovery selected the wrong journal or actor:\n{evidence}"
        );
    }
}

#[test]
fn lazy_recovery_agent_mixed_submission_pins_journal() {
    recovery_case(false, false, false);
}
#[test]
fn lazy_recovery_agent_mixed_submission_pins_ambient_journal() {
    recovery_case(false, false, true);
}
#[test]
fn lazy_recovery_human_submission_pins_journal() {
    recovery_case(true, false, false);
}
#[test]
fn lazy_recovery_human_submission_pins_ambient_journal() {
    recovery_case(true, false, true);
}
#[test]
fn lazy_recovery_prior_durable_ref_on_second_record_failure_pins_journal() {
    recovery_case(false, true, false);
}
#[test]
fn lazy_recovery_prior_durable_ref_on_second_record_failure_pins_ambient_journal() {
    recovery_case(false, true, true);
}
