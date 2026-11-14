//! The opencode caller role witness (opencode design D2, design review M2): the plugin's session
//! variables, the pid echo against `OPENCODE_PID`, and the nested-opencode inheritance cases,
//! both on the witness and through the whole caller derivation. Included from
//! `src/harness/opencode/witness.rs`.
use super::*;
use crate::harness::caller_env::{EnvInputs, EnvRefusal, derive};
use crate::ports::{ProcessInfo, ProcessRecord};
use crate::protocol::authority::{CallerRole, LivenessFingerprint};
use std::collections::HashMap;

const TOP: &str = "ses_1a2B3c4D5e6F7g8H9i0J";
const CHILD: &str = "ses_9z8Y7x6W5v4U3t2S1r0Q";
const INNER: &str = "ses_5i5I5i5I5i5I5i5I5i5I";

fn lookup(env: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
    let map: HashMap<String, String> = env
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    move |name: &str| map.get(name).cloned()
}

fn witness(session: &str, env: &[(&str, &str)]) -> CallerSession {
    caller_session(session, &lookup(env))
}

#[test]
fn session_ids_are_ses_and_one_to_sixty_four_alphanumerics() {
    for id in [TOP, "ses_a", &format!("ses_{}", "Z9".repeat(32))] {
        assert!(valid_session_id(id), "{id}");
    }
    for id in [
        "",
        "ses_",
        "ses",
        "SES_a",
        "ses_a-b",
        "ses_a b",
        "ses_\u{e9}",
        &format!("ses_{}", "a".repeat(65)),
    ] {
        assert!(!valid_session_id(id), "{id:?}");
    }
}

#[test]
fn an_empty_or_absent_parent_is_top_level() {
    let pid = [(PID_VAR, "41"), (PID_ECHO_VAR, "41")];
    assert_eq!(witness(TOP, &pid), CallerSession::TopLevel);
    assert_eq!(
        witness(TOP, &[pid[0], pid[1], (PARENT_VAR, "")]),
        CallerSession::TopLevel
    );
}

#[test]
fn a_valid_parent_is_a_subagent_of_it() {
    assert_eq!(
        witness(
            CHILD,
            &[(PID_VAR, "41"), (PID_ECHO_VAR, "41"), (PARENT_VAR, TOP)]
        ),
        CallerSession::Subagent { parent: TOP.into() }
    );
}

#[test]
fn any_other_parent_is_a_malformed_marker() {
    for parent in ["?", "ses_", "nonsense", CHILD] {
        assert_eq!(
            witness(
                CHILD,
                &[(PID_VAR, "41"), (PID_ECHO_VAR, "41"), (PARENT_VAR, parent)]
            ),
            CallerSession::MalformedMarker { var: PARENT_VAR },
            "{parent}"
        );
    }
}

#[test]
fn the_pid_echo_must_equal_opencode_pid() {
    for env in [
        vec![(PID_VAR, "41")],
        vec![(PID_ECHO_VAR, "41")],
        vec![],
        vec![(PID_VAR, "41"), (PID_ECHO_VAR, "42")],
        vec![(PID_VAR, ""), (PID_ECHO_VAR, "")],
    ] {
        assert!(
            matches!(witness(TOP, &env), CallerSession::Unattributable(_)),
            "{env:?}"
        );
    }
    assert!(matches!(
        witness("not-a-session", &[(PID_VAR, "41"), (PID_ECHO_VAR, "41")]),
        CallerSession::Unattributable(_)
    ));
}

// ---- Through the whole derivation: the nested-opencode inheritance cases (review M2).

struct FakeTable(Vec<ProcessRecord>);
impl ProcessInfo for FakeTable {
    fn process(&self, pid: u32) -> Option<ProcessRecord> {
        self.0.iter().find(|p| p.pid == pid).cloned()
    }
}

fn rec(pid: u32, parent: u32, start: u64, argv: &[&str]) -> ProcessRecord {
    ProcessRecord {
        pid,
        parent: Some(parent),
        start,
        argv: argv.iter().map(|a| a.to_string()).collect(),
    }
}

/// `herdr-threads` (100) under the outer opencode (80), or under an inner opencode (70) that
/// a shell call of the outer one started.
fn table(nested: bool) -> FakeTable {
    let mut rows = vec![rec(80, 1, 800, &["opencode"])];
    if nested {
        rows.push(rec(70, 80, 700, &["opencode", "--pure"]));
        rows.push(rec(100, 70, 1000, &["herdr-threads", "inbox"]));
    } else {
        rows.push(rec(100, 80, 1000, &["herdr-threads", "inbox"]));
    }
    FakeTable(rows)
}

fn run(
    table: &FakeTable,
    env: &[(&str, &str)],
) -> Result<crate::harness::caller_env::EnvClaim, EnvRefusal> {
    let env = lookup(env);
    derive(&EnvInputs {
        env: &env,
        self_pid: 100,
        agent_id: None,
        processes: table,
    })
}

#[test]
fn a_top_level_call_claims_its_session_with_the_opencode_fingerprint() {
    let claim = run(
        &table(false),
        &[
            (SESSION_VAR, TOP),
            (PARENT_VAR, ""),
            (PID_ECHO_VAR, "80"),
            (PID_VAR, "80"),
        ],
    )
    .unwrap();
    assert_eq!(
        claim.harness,
        crate::protocol::authority::Harness::Agent(
            crate::harness::registry::builtins()
                .agent("opencode")
                .unwrap()
        )
    );
    assert_eq!(claim.native_session.as_str(), TOP);
    assert_eq!(claim.role, CallerRole::TopLevel);
    assert_eq!(claim.agent_id, None);
    assert_eq!(claim.provenance, "cooperative_env_session");
    assert_eq!(
        claim.fingerprint,
        LivenessFingerprint {
            pid: 80,
            start: 800
        }
    );
}

#[test]
fn a_child_call_claims_a_subagent_binding_of_the_root_parent() {
    let claim = run(
        &table(false),
        &[
            (SESSION_VAR, CHILD),
            (PARENT_VAR, TOP),
            (PID_ECHO_VAR, "80"),
            (PID_VAR, "80"),
        ],
    )
    .unwrap();
    assert_eq!(claim.native_session.as_str(), TOP);
    assert_eq!(claim.role, CallerRole::Subagent);
    assert_eq!(claim.agent_id.as_ref().map(|a| a.as_str()), Some(CHILD));
    assert_eq!(claim.fingerprint.pid, 80);
}

#[test]
fn no_session_variable_is_refused_naming_it() {
    // The plugin is missing (not set up, or `--pure`).
    assert_eq!(
        run(&table(false), &[(PID_VAR, "80")]),
        Err(EnvRefusal::MissingSession {
            harness: "opencode",
            var: SESSION_VAR,
        })
    );
}

/// A nested `opencode --pure` sets none of the variables but exports its own `OPENCODE_PID`:
/// the outer session's variables are refused, never claimed by the inner process.
#[test]
fn a_nested_pure_opencode_never_inherits_the_outer_session() {
    for parent in ["", TOP] {
        let session = if parent.is_empty() { TOP } else { CHILD };
        let result = run(
            &table(true),
            &[
                (SESSION_VAR, session),
                (PARENT_VAR, parent),
                (PID_ECHO_VAR, "80"),
                (PID_VAR, "70"),
            ],
        );
        assert!(
            matches!(
                result,
                Err(EnvRefusal::UnattributedSession {
                    harness: "opencode",
                    ..
                })
            ),
            "{parent:?}: {result:?}"
        );
    }
}

/// A nested opencode with the plugin overwrites all three variables on its own calls, so a stale
/// parent from an outer child session never makes its top-level session a child.
#[test]
fn a_nested_plugin_opencode_overwrites_a_stale_parent() {
    let claim = run(
        &table(true),
        &[
            (SESSION_VAR, INNER),
            (PARENT_VAR, ""),
            (PID_ECHO_VAR, "70"),
            (PID_VAR, "70"),
        ],
    )
    .unwrap();
    assert_eq!(claim.native_session.as_str(), INNER);
    assert_eq!(claim.role, CallerRole::TopLevel);
    assert_eq!(
        claim.fingerprint,
        LivenessFingerprint {
            pid: 70,
            start: 700
        }
    );
}

#[test]
fn an_unknown_parent_fails_toward_child() {
    assert_eq!(
        run(
            &table(false),
            &[
                (SESSION_VAR, CHILD),
                (PARENT_VAR, "?"),
                (PID_ECHO_VAR, "80"),
                (PID_VAR, "80"),
            ],
        ),
        Err(EnvRefusal::MalformedSubagentMarker {
            harness: "opencode",
            var: PARENT_VAR,
        })
    );
}
