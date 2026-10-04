//! `pane_not_found` names the precedence, and the autodetect probe is bounded
//! (root §B10 Decision 3). Mounted from `src/cli/panes.rs`.

use super::*;
use crate::cli::setup::{
    AUTODETECT_PROBE_TIMEOUT, DetectInputs, PROBE_TIMEOUT, detect_host_endpoint, detect_state_dir,
};
use crate::protocol::results::ErrorCode;
use std::{
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

#[test]
fn pane_not_found_hint_names_the_scoped_matching_rule() {
    let error = resolve_selector(
        &PaneSelector {
            pane: Some("nope".into()),
            ..Default::default()
        },
        &topology(),
        Some("w1:p1"),
    )
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::NotFound);
    assert!(error.detail.contains(NAME_PRECEDENCE));
    assert!(error.detail.contains("herdr pane current"));
}

#[test]
fn an_exact_pane_id_beats_a_label_that_looks_like_it() {
    let mut topology = topology();
    topology.panes[1].label = Some("w1:p1".into());
    let selector = PaneSelector {
        pane: Some("w1:p1".into()),
        ..Default::default()
    };
    assert_eq!(
        resolve_selector(&selector, &topology, None)
            .unwrap()
            .as_str(),
        "w1:p1"
    );
}

fn hung_herdr(dir: &Path) -> PathBuf {
    let bin = dir.join("herdr");
    std::fs::write(&bin, "#!/bin/sh\nexec sleep 30\n").unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    bin
}

/// Kills: detection waiting the old 5 s query timeout on a Herdr that never
/// answers. A hung `herdr` must fail both probes within the short bound (plus
/// scheduling slack), with the pass-it-explicitly remedy in the message.
#[test]
fn autodetect_probe_is_bounded() {
    assert!(
        AUTODETECT_PROBE_TIMEOUT <= Duration::from_millis(2_000),
        "the probe bound is short: {AUTODETECT_PROBE_TIMEOUT:?}"
    );
    let dir = std::env::temp_dir().join(format!("ht-hung-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&dir).unwrap();
    let inputs = DetectInputs {
        herdr: Some(hung_herdr(&dir)),
        home: Some(dir.join("home")),
        xdg_state_home: None,
    };
    // Other unit tests run with a generous probe; this one runs with the
    // production bound.
    PROBE_TIMEOUT.with(|probe| probe.set(AUTODETECT_PROBE_TIMEOUT));
    let started = Instant::now();
    let state = detect_state_dir(&inputs).unwrap_err();
    let first = started.elapsed();
    let started = Instant::now();
    let host = detect_host_endpoint(&inputs).unwrap_err();
    let second = started.elapsed();
    std::fs::remove_dir_all(&dir).unwrap();
    assert!(
        first < Duration::from_millis(4_000) && second < Duration::from_millis(4_000),
        "hung probes took {first:?} and {second:?}, not the 5 s query timeout"
    );
    assert!(
        first >= AUTODETECT_PROBE_TIMEOUT - Duration::from_millis(100),
        "the probe gave up after {first:?}, before its {AUTODETECT_PROBE_TIMEOUT:?} bound"
    );
    assert!(state.contains("did not answer"), "{state}");
    assert!(host.contains("did not answer"), "{host}");
}

/// Kills: docs/agent-usage.md drifting from the probe bound (ht-p03.108).
#[test]
fn agent_usage_states_the_probe_bound() {
    let doc =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/agent-usage.md"))
            .unwrap();
    let secs = AUTODETECT_PROBE_TIMEOUT.as_secs();
    assert_eq!(
        AUTODETECT_PROBE_TIMEOUT,
        Duration::from_secs(secs),
        "whole seconds"
    );
    assert!(
        doc.contains(&format!("short probe timeout ({secs} s)")),
        "agent-usage.md does not say {secs} s"
    );
}

#[test]
fn scoped_invite_and_recipient_reads_parse() {
    for argv in [
        vec!["ht", "invite", "t123", "--tab", "tryout", "--pane", "alice"],
        vec![
            "ht", "inbox", "--space", "project", "--tab", "tryout", "--pane", "alice",
        ],
        vec!["ht", "seat", "resolve"],
        vec![
            "ht",
            "send",
            "t123",
            "--body",
            "work",
            "--tab",
            "tryout",
            "--require-ack-pane",
            "alice",
        ],
    ] {
        assert!(
            super::super::commands::parse_argv(argv.clone()).is_ok(),
            "{argv:?}"
        );
    }
}

#[test]
fn unavailable_host_refuses_names() {
    let mut parsed =
        super::super::commands::parse_argv(["ht", "seat", "resolve", "--pane", "alice"]).unwrap();
    let error = resolve_cli_targets(
        &mut parsed,
        || Err(ApiError::host_unavailable("down")),
        || panic!("host read already failed"),
    )
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::HostUnavailable);
}

fn topology_raw() -> serde_json::Value {
    let p = |id: &str, tab: &str, label: &str| {
        serde_json::json!({"pane_id":id,"terminal_id":id,
        "workspace_id":"w1","tab_id":tab,"label":label,"focused":false,"agent_status":"idle","revision":1})
    };
    serde_json::json!({"result":{"type":"session_snapshot","snapshot":{
        "version":"0.9.1","protocol":22,"layouts":[],
        "workspaces":[{"workspace_id":"w1","label":"project"}],
        "tabs":[{"tab_id":"w1:t1","workspace_id":"w1","label":"main"},
                {"tab_id":"w1:t2","workspace_id":"w1","label":"tryout"}],
        "panes":[p("w1:p1","w1:t1","alice"),p("w1:p2","w1:t2","alice"),p("w1:p3","w1:t2","bob\n--pane x")],
        "agents":[{"pane_id":"w1:p3","name":"alice"}]
    }}})
}

fn topology() -> crate::host::observation::HostTopology {
    crate::host::observation::normalize_topology(&topology_raw().to_string()).unwrap()
}

#[test]
fn selector_scopes_names_and_unions_live_agent_names() {
    let mut topology = topology();
    let selector = PaneSelector {
        pane: Some("alice".into()),
        ..Default::default()
    };
    assert_eq!(
        resolve_selector(&selector, &topology, Some("w1:p1"))
            .unwrap()
            .as_str(),
        "w1:p1"
    );
    let selector = PaneSelector {
        tab: Some("tryout".into()),
        ..selector
    };
    let error = resolve_selector(&selector, &topology, Some("w1:p1")).unwrap_err();
    assert_eq!(error.code, ErrorCode::Conflict);
    assert!(error.detail.contains("w1:p2") && error.detail.contains("w1:p3"));
    assert!(!error.detail.contains('\n'));
    topology.panes[2].agent_names.clear();
    assert_eq!(
        resolve_selector(&selector, &topology, Some("w1:p1"))
            .unwrap()
            .as_str(),
        "w1:p2"
    );
}

#[test]
fn selector_defaults_use_live_caller_and_never_carry_child_across_parents() {
    let topology = topology();
    assert_eq!(
        resolve_selector(&PaneSelector::default(), &topology, Some("w1:p3"))
            .unwrap()
            .as_str(),
        "w1:p3"
    );
    assert!(resolve_selector(&PaneSelector::default(), &topology, Some("w1:p99")).is_err());
    let selector = PaneSelector {
        tab: Some("tryout".into()),
        ..Default::default()
    };
    assert_eq!(
        resolve_selector(&selector, &topology, Some("w1:p1"))
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    let selector = PaneSelector {
        tab: Some("main".into()),
        ..Default::default()
    };
    assert_eq!(
        resolve_selector(&selector, &topology, Some("w1:p3"))
            .unwrap()
            .as_str(),
        "w1:p1"
    );
}

#[test]
fn explicit_pane_id_checks_parents_and_tab_id_supplies_space() {
    let topology = topology();
    let selector = PaneSelector {
        tab: Some("tryout".into()),
        pane: Some("w1:p1".into()),
        ..Default::default()
    };
    assert!(resolve_selector(&selector, &topology, Some("w1:p1")).is_err());
    let selector = PaneSelector {
        tab: Some("w1:t1".into()),
        pane: Some("alice".into()),
        ..Default::default()
    };
    assert_eq!(
        resolve_selector(&selector, &topology, None)
            .unwrap()
            .as_str(),
        "w1:p1"
    );
    assert!(
        resolve_selector(
            &PaneSelector {
                pane: Some("alice".into()),
                ..Default::default()
            },
            &topology,
            None
        )
        .is_err()
    );
}

#[test]
fn quoted_exact_labels_parse_without_becoming_wire_ids() {
    for argv in [
        vec!["ht", "seat", "resolve", "--pane", "Alice Smith"],
        vec!["ht", "launch", "--pane", "Équipe", "--kind", "codex"],
        vec![
            "ht",
            "inbox",
            "--cooperative-seat",
            "s123",
            "--cooperative-target",
            "Alice Smith",
            "--cooperative-harness",
            "codex",
            "--cooperative-role",
            "top-level",
        ],
    ] {
        assert!(
            super::super::commands::parse_argv(argv.clone()).is_ok(),
            "{argv:?}"
        );
    }
}

#[test]
fn cooperative_caller_scope_stays_independent_of_recipient_scope() {
    let mut parsed = super::super::commands::parse_argv([
        "ht",
        "invite",
        "t123",
        "--tab",
        "tryout",
        "--pane",
        "bob\n--pane x",
        "--cooperative-seat",
        "sender",
        "--cooperative-target",
        "alice",
        "--cooperative-harness",
        "codex",
        "--cooperative-role",
        "top-level",
    ])
    .unwrap();
    resolve_cli_targets(
        &mut parsed,
        || Ok(topology()),
        || Ok(Some(HostTargetId::new("w1:p1"))),
    )
    .unwrap();
    let selection = parsed.cooperative.unwrap();
    assert_eq!(selection.target.as_str(), "w1:p1");
    assert_eq!(selection.seat.as_str(), "sender");
    assert_eq!(
        parsed.pane_selector.unwrap().direct_id().unwrap().as_str(),
        "w1:p3"
    );
}

#[test]
fn resolution_freezes_live_moved_caller_and_direct_ids_skip_host_reads() {
    let mut parsed = super::super::commands::parse_argv(["ht", "seat", "resolve"]).unwrap();
    resolve_cli_targets(
        &mut parsed,
        || Ok(topology()),
        || Ok(Some(HostTargetId::new("w1:p3"))),
    )
    .unwrap();
    assert!(
        matches!(parsed.action, CliAction::Mutation(MutationSpec::Resolve(ref pane)) if pane.as_str() == "w1:p3")
    );
    resolve_cli_targets(
        &mut parsed,
        || panic!("already frozen"),
        || panic!("already frozen"),
    )
    .unwrap();
    let mut parsed =
        super::super::commands::parse_argv(["ht", "seat", "resolve", "--pane", "w9:p9"]).unwrap();
    resolve_cli_targets(&mut parsed, || panic!("direct ID"), || panic!("direct ID")).unwrap();
    assert!(
        matches!(parsed.action, CliAction::Mutation(MutationSpec::Resolve(ref pane)) if pane.as_str() == "w9:p9")
    );
}

#[test]
fn runtime_parent_only_defaults_use_live_caller_for_equivalent_ids_and_labels() {
    use std::cell::Cell;
    let calls = Cell::new(0);
    // tryout has two panes: the inherited hint names main, but the live caller moved.
    for parents in [
        vec!["--tab", "tryout"],
        vec!["--tab", "w1:t2"],
        vec!["--space", "project", "--tab", "tryout"],
        vec!["--space", "w1", "--tab", "w1:t2"],
        vec!["--space", "project"],
    ] {
        for command in [vec!["ht", "seat", "resolve"], vec!["ht", "inbox"]] {
            let argv: Vec<_> = command.into_iter().chain(parents.iter().copied()).collect();
            let mut parsed = super::super::commands::parse_argv(argv.clone()).unwrap();
            resolve_cli_targets(
                &mut parsed,
                || Ok(topology()),
                || {
                    calls.set(calls.get() + 1);
                    Ok(Some(HostTargetId::new("w1:p3")))
                },
            )
            .unwrap_or_else(|error| panic!("{argv:?}: {error:?}"));
            assert_eq!(
                parsed.pane_selector.unwrap().direct_id().unwrap().as_str(),
                "w1:p3",
                "{argv:?} must retain the live caller's own pane"
            );
        }
    }
    assert_eq!(calls.get(), 10, "one caller lookup per unresolved command");
}

#[test]
fn runtime_parent_only_defaults_preserve_different_parent_and_absent_caller_rules() {
    for (parents, caller, want) in [
        (vec!["--tab", "w1:t1"], Some("w1:p3"), Ok("w1:p1")),
        (
            vec!["--space", "project", "--tab", "main"],
            None,
            Ok("w1:p1"),
        ),
        (
            vec!["--tab", "w1:t2"],
            Some("w1:p1"),
            Err(ErrorCode::Conflict),
        ),
        (
            vec!["--space", "project", "--tab", "tryout"],
            None,
            Err(ErrorCode::Conflict),
        ),
        (
            vec!["--tab", "w1:t2"],
            Some("w1:p99"),
            Err(ErrorCode::Conflict),
        ),
        (
            vec!["--tab", "w1:t99"],
            Some("w1:p3"),
            Err(ErrorCode::NotFound),
        ),
    ] {
        let argv: Vec<_> = ["ht", "inbox"].into_iter().chain(parents).collect();
        let mut parsed = super::super::commands::parse_argv(argv.clone()).unwrap();
        let result = resolve_cli_targets(
            &mut parsed,
            || Ok(topology()),
            || Ok(caller.map(HostTargetId::new)),
        );
        match want {
            Ok(target) => {
                result.unwrap_or_else(|error| panic!("{argv:?}: {error:?}"));
                assert_eq!(
                    parsed.pane_selector.unwrap().direct_id().unwrap().as_str(),
                    target
                );
            }
            Err(code) => assert_eq!(result.unwrap_err().code, code, "{argv:?}"),
        }
    }
    let mut parsed = super::super::commands::parse_argv(["ht", "inbox", "--tab", "w1:t2"]).unwrap();
    assert_eq!(
        resolve_cli_targets(
            &mut parsed,
            || Ok(topology()),
            || Err(ApiError::host_unavailable("live caller unavailable")),
        )
        .unwrap_err()
        .code,
        ErrorCode::HostUnavailable,
        "a failed live caller read cannot guess a child"
    );
}

#[test]
fn runtime_qualified_explicit_panes_keep_parent_validation_without_caller_lookup() {
    for (pane, want) in [
        ("w1:p3", Ok("w1:p3")),
        ("w1:p1", Err(ErrorCode::InvalidRequest)),
        ("w1:p99", Err(ErrorCode::NotFound)),
    ] {
        let mut parsed =
            super::super::commands::parse_argv(["ht", "inbox", "--tab", "w1:t2", "--pane", pane])
                .unwrap();
        let result = resolve_cli_targets(
            &mut parsed,
            || Ok(topology()),
            || panic!("explicit pane ID"),
        );
        match want {
            Ok(target) => {
                result.unwrap();
                assert_eq!(
                    parsed.pane_selector.unwrap().direct_id().unwrap().as_str(),
                    target
                );
            }
            Err(code) => assert_eq!(result.unwrap_err().code, code),
        }
    }
}

#[test]
fn current_pane_normalization_uses_current_response_not_focus() {
    let raw = serde_json::json!({"result":{"type":"pane_current","pane":{"pane_id":"w1:p3","focused":false}}});
    assert_eq!(
        crate::host::observation::normalize_current_pane(&raw.to_string())
            .unwrap()
            .as_str(),
        "w1:p3"
    );
}

#[test]
fn parent_only_read_scope_cannot_become_all_instance_directory() {
    assert!(
        super::super::commands::parse_argv(["ht", "thread", "list", "--all", "--tab", "tryout"])
            .is_err()
    );
}

#[test]
fn parser_rejects_mixed_durable_seat_and_pane_scope_and_requires_target_intent() {
    for argv in [
        vec!["ht", "invite", "t123", "--seat", "s123", "--tab", "tryout"],
        vec!["ht", "inbox", "--seat", "s123", "--pane", "alice"],
        vec![
            "ht",
            "seat",
            "resolve",
            "--new-seat",
            "--operator",
            "--tab",
            "tryout",
        ],
        vec![
            "ht",
            "seat",
            "rebind",
            "s123",
            "--operator",
            "--tab",
            "tryout",
        ],
        vec!["ht", "launch", "--kind", "codex", "--tab", "tryout"],
    ] {
        assert!(
            super::super::commands::parse_argv(argv.clone()).is_err(),
            "{argv:?}"
        );
    }
}

#[test]
fn aliases_deduplicate_and_stay_inside_the_selected_tab() {
    let mut topology = topology();
    topology.panes[0].agent_names = vec!["alice".into()];
    let selector = PaneSelector {
        pane: Some("alice".into()),
        ..Default::default()
    };
    assert_eq!(
        resolve_selector(&selector, &topology, Some("w1:p1"))
            .unwrap()
            .as_str(),
        "w1:p1"
    );
    let alias = PaneSelector {
        pane: Some("main".into()),
        ..Default::default()
    };
    assert_eq!(
        resolve_selector(&alias, &topology, Some("w1:p1"))
            .unwrap()
            .as_str(),
        "w1:p1"
    );
    assert_eq!(
        resolve_selector(&alias, &topology, Some("w1:p3"))
            .unwrap_err()
            .code,
        ErrorCode::NotFound
    );
}

#[test]
fn explicit_different_space_uses_sole_children_and_rejects_mismatched_ids() {
    use crate::host::observation::{TopologyPane, TopologySpace, TopologyTab};
    let mut topology = topology();
    topology.spaces.push(TopologySpace {
        id: "w2".into(),
        label: Some("other".into()),
    });
    topology.tabs.push(TopologyTab {
        id: "w2:t1".into(),
        space: "w2".into(),
        label: Some("tryout".into()),
    });
    topology.panes.push(TopologyPane {
        target: HostTargetId::new("w2:p1"),
        space: "w2".into(),
        tab: "w2:t1".into(),
        label: Some("alice".into()),
        agent_names: vec![],
    });
    let selector = PaneSelector {
        space: Some("other".into()),
        ..Default::default()
    };
    assert_eq!(
        resolve_selector(&selector, &topology, Some("w1:p3"))
            .unwrap()
            .as_str(),
        "w2:p1"
    );
    assert_eq!(
        resolve_selector(&selector, &topology, None)
            .unwrap()
            .as_str(),
        "w2:p1"
    );
    let selector = PaneSelector {
        pane: Some("w1:p1".into()),
        ..selector
    };
    assert!(resolve_selector(&selector, &topology, Some("w1:p3")).is_err());
}

#[test]
fn dispatch_never_submits_unresolved_names_or_recipient_locators() {
    struct Backend(usize);
    impl super::super::commands::CliBackend for Backend {
        fn call(
            &mut self,
            _: crate::protocol::commands::Command,
            _: &crate::protocol::output::OutputSpec,
        ) -> Result<crate::protocol::results::CommandResult, ApiError> {
            self.0 += 1;
            Ok(crate::protocol::results::CommandResult::SeatResolved(
                crate::protocol::ids::SeatId::new("s123"),
            ))
        }
    }
    for argv in [
        vec!["ht", "seat", "resolve", "--pane", "Alice Smith"],
        vec!["ht", "inbox", "--pane", "w1:p2"],
    ] {
        let parsed = super::super::commands::parse_argv(argv).unwrap();
        let mut backend = Backend(0);
        let result = super::super::commands::dispatch(
            parsed,
            &mut backend,
            None,
            Some(crate::protocol::ids::OperationId::new("op123")),
        );
        assert!(result.is_err(), "must refuse unprepared target metadata");
        assert_eq!(backend.0, 0);
    }
}

#[test]
fn topology_refuses_incoherent_parent_and_agent_rows() {
    let mut wrong_parent = topology_raw();
    wrong_parent["result"]["snapshot"]["tabs"][0]["workspace_id"] = serde_json::json!("w99");
    let mut wrong_agent = topology_raw();
    wrong_agent["result"]["snapshot"]["agents"][0]["pane_id"] = serde_json::json!("w1:p99");
    let mut duplicate_tab = topology_raw();
    duplicate_tab["result"]["snapshot"]["tabs"][1]["tab_id"] = serde_json::json!("w1:t1");
    for raw in [wrong_parent, wrong_agent, duplicate_tab] {
        assert_eq!(
            crate::host::observation::normalize_topology(&raw.to_string())
                .unwrap_err()
                .code,
            ErrorCode::StaleHostObservation
        );
    }
}

#[test]
fn default_reads_use_daemon_caller_mapping_without_host_locator_reads() {
    for argv in [
        vec!["ht", "inbox"],
        vec!["ht", "pending-receipts"],
        vec!["ht", "diagnostics"],
        vec!["ht", "warnings"],
        vec!["ht", "thread", "list"],
    ] {
        let mut parsed = super::super::commands::parse_argv(argv).unwrap();
        resolve_cli_targets(
            &mut parsed,
            || panic!("default caller reads do not need live topology"),
            || panic!("default caller reads use HERDR_PANE_ID"),
        )
        .unwrap();
        assert!(parsed.caller_read_default);
        assert!(parsed.pane_selector.is_none());
    }
}
