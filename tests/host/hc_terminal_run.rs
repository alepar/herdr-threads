//! `HcHost` as a `CreateTargetPort`: the `terminal.run` request, labels,
//! placements and outcome mapping against `fake_hc`.

use super::labels::{LABEL_BOOTSTRAP, LABEL_HT, LABEL_SPACE, LABEL_TAB};
use super::*;
use crate::ports::{
    CreateTargetOutcome, CreateTargetPort, CreateTargetRequest, HcPeer, HostWitness, Placement,
};
use crate::protocol::ids::HostCallId;
use crate::protocol::time::{Cancellation, MonoInstant};
use crate::test_support::fake_hc::{FakeHc, FakeTerminal, RunFailure};
use serde_json::{Value, json};

fn context() -> HostCallContext {
    HostCallContext {
        budget: CallBudget {
            deadline: MonoInstant(10_000),
            cancellation: Cancellation::default(),
        },
        expected_boot: None,
        expected_epoch: None,
    }
}

fn gate() -> Vec<String> {
    [
        "herdr-threads",
        "internal",
        "launch-gate",
        "--bootstrap",
        "op:1",
        "--",
        "claude",
    ]
    .map(String::from)
    .to_vec()
}

fn setup() -> (FakeHc, HcHost, HostWitness) {
    let fake = FakeHc::start();
    fake.add_terminal(FakeTerminal::live("t1"));
    let client = HcClient::new(fake.socket());
    let hello = HostWitnessProbe::hello(&client);
    let host = HcHost::new(client)
        .with_gate_argv(gate())
        .with_run_deadline(std::time::Duration::from_millis(400));
    (fake, host, hello)
}

struct HostWitnessProbe;
impl HostWitnessProbe {
    fn hello(client: &HcClient) -> HostWitness {
        let reply = HcHost::run(client.hello()).unwrap();
        HostWitness::HcCore {
            core_id: reply.value.core.id,
            peer: HcPeer {
                pid: reply.witness.peer_pid,
                start_seconds: reply.witness.start_seconds,
                uid: reply.witness.peer_uid,
            },
        }
    }
}

fn request(placement: Placement, witness: HostWitness) -> CreateTargetRequest {
    CreateTargetRequest {
        correlation: HostCallId::new("corr-1"),
        placement,
        cwd: "/tmp".into(),
        label: "worker".into(),
        focus: false,
        env: [("K".to_owned(), "V".to_owned())].into(),
        directory_context: None,
        expected_witness: witness,
    }
}

fn new_tab(witness: HostWitness) -> CreateTargetRequest {
    request(
        Placement::NewTab {
            space: HostTargetId::new("sp-1"),
        },
        witness,
    )
}

fn new_terminal(witness: HostWitness) -> CreateTargetRequest {
    request(
        Placement::NewTerminal {
            parent: HostTargetId::new("t1"),
        },
        witness,
    )
}

fn run_params(fake: &FakeHc) -> Value {
    let calls = fake.calls_to("terminal.run");
    assert_eq!(calls.len(), 1, "exactly one terminal.run: {calls:?}");
    calls[0].params.clone()
}

#[test]
fn terminal_run_params() {
    let (fake, host, witness) = setup();
    let CreateTargetOutcome::Created(created) = host.create_target(&new_tab(witness), &context())
    else {
        panic!("expected Created");
    };
    let p = run_params(&fake);
    assert_eq!(p["no_notify"], true);
    assert_eq!(p["outlive_parent"], true);
    assert_eq!(p["no_parent"], true);
    assert_eq!(p["keep"], json!(false), "keep is present and false (R43)");
    for absent in [
        "caller",
        "name",
        "first_message",
        "template",
        "wait",
        "rm",
        "line",
    ] {
        assert!(p.get(absent).is_none(), "{absent} must not be sent: {p}");
    }
    assert_eq!(p["argv"], json!(gate()));
    assert_eq!(p["cwd"], "/tmp");
    assert_eq!(p["env"], json!({"K": "V"}));
    // The recorded terminal is the final-on-finish kind and reports no notes.
    let t = HcHost::run(HcClient::new(fake.socket()).terminal_get(created.target.as_str()))
        .unwrap()
        .value;
    assert!(!t.keep);
}

#[test]
fn new_tab_labels() {
    let (fake, host, witness) = setup();
    let first = host.create_target(&new_tab(witness.clone()), &context());
    let second = host.create_target(&new_tab(witness), &context());
    assert!(matches!(first, CreateTargetOutcome::Created(_)));
    assert!(matches!(second, CreateTargetOutcome::Created(_)));
    let calls = fake.calls_to("terminal.run");
    let labels = |i: usize| calls[i].params["labels"].as_object().unwrap().clone();
    let (a, b) = (labels(0), labels(1));
    let keys: std::collections::BTreeSet<_> = a.keys().map(String::as_str).collect();
    assert_eq!(
        keys,
        [LABEL_SPACE, LABEL_TAB, LABEL_BOOTSTRAP, LABEL_HT].into()
    );
    assert_eq!(a[LABEL_SPACE], "sp-1");
    assert_eq!(a[LABEL_BOOTSTRAP], "corr-1");
    assert_eq!(a[LABEL_HT], "worker");
    assert!(!a[LABEL_TAB].as_str().unwrap().is_empty());
    assert_ne!(a[LABEL_TAB], b[LABEL_TAB], "herdr.tab is fresh per launch");
    assert!(calls[0].params.get("parent").is_none());
}

#[test]
fn new_terminal_no_app_labels_parent_caller() {
    let (fake, host, witness) = setup();
    let CreateTargetOutcome::Created(created) =
        host.create_target(&new_terminal(witness), &context())
    else {
        panic!("expected Created");
    };
    let p = run_params(&fake);
    assert_eq!(p["parent"], "t1");
    let labels = p["labels"].as_object().unwrap();
    assert!(!labels.contains_key(LABEL_SPACE) && !labels.contains_key(LABEL_TAB));
    assert!(
        labels.is_empty(),
        "NewTerminal sends no app labels: {labels:?}"
    );
    // The new terminal is a child of t1 in the core and the witness validates.
    let t = HcHost::run(HcClient::new(fake.socket()).terminal_get(created.target.as_str()))
        .unwrap()
        .value;
    assert_eq!(t.address, "t2");
    assert_eq!(created.terminal.as_str(), "t2");
    assert!(created.validate().is_ok());
    assert!(
        created
            .host_incarnation
            .as_str()
            .starts_with("hc-core:core-0001:pid=")
    );
}

#[test]
fn pre_record_rpc_error_not_submitted() {
    for kind in ["unavailable", "invalid_params", "config"] {
        let (fake, host, witness) = setup();
        fake.fail_next_run(RunFailure::Error(kind));
        let outcome = host.create_target(&new_tab(witness), &context());
        assert!(
            matches!(outcome, CreateTargetOutcome::NotSubmitted(_)),
            "{kind}: {outcome:?}"
        );
        assert_eq!(fake.calls_to("terminal.run").len(), 1);
    }
}

#[test]
fn refusals_before_any_call_are_not_submitted() {
    let (fake, host, witness) = setup();
    // A different core id or peer than the caller expects: nothing is run.
    let HostWitness::HcCore { peer, .. } = &witness else {
        unreachable!()
    };
    let wrong_core = HostWitness::HcCore {
        core_id: "core-other".into(),
        peer: peer.clone(),
    };
    let outcome = host.create_target(&new_tab(wrong_core), &context());
    assert!(
        matches!(outcome, CreateTargetOutcome::NotSubmitted(_)),
        "{outcome:?}"
    );
    // A host with no gate argv creates nothing.
    let bare = HcHost::new(HcClient::new(fake.socket()));
    let outcome = bare.create_target(&new_tab(witness), &context());
    assert!(
        matches!(outcome, CreateTargetOutcome::NotSubmitted(_)),
        "no gate argv"
    );
    assert!(fake.calls_to("terminal.run").is_empty());
}

#[test]
fn spawn_failed_timeout_disconnect_outcome_unknown() {
    for failure in [
        RunFailure::SpawnFailed,
        RunFailure::Hang,
        RunFailure::Disconnect,
    ] {
        let (fake, host, witness) = setup();
        fake.fail_next_run(failure.clone());
        let outcome = host.create_target(&new_tab(witness), &context());
        assert!(
            matches!(outcome, CreateTargetOutcome::OutcomeUnknown(_)),
            "{failure:?}: {outcome:?}"
        );
    }
}

#[test]
fn recorded_unknown_never_reclassified_by_bootstrap_label() {
    let (fake, host, witness) = setup();
    fake.fail_next_run(RunFailure::Disconnect);
    let outcome = host.create_target(&new_tab(witness), &context());
    assert!(matches!(outcome, CreateTargetOutcome::OutcomeUnknown(_)));
    // The core now holds a terminal carrying our ht.bootstrap hint ...
    let t = HcHost::run(HcClient::new(fake.socket()).terminal_get("t2"))
        .unwrap()
        .value;
    assert_eq!(
        t.labels.get(LABEL_BOOTSTRAP).map(String::as_str),
        Some("corr-1")
    );
    // ... and the host neither looked for it nor changed its answer.
    let methods: Vec<String> = fake.calls().into_iter().map(|c| c.method).collect();
    assert_eq!(
        methods
            .iter()
            .filter(|m| *m != "hello" && *m != "terminal.get")
            .collect::<Vec<_>>(),
        ["terminal.run"],
        "{methods:?}"
    );
}
