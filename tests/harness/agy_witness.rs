//! agy `local_evidence` (D5) on fixture layouts built in temp HOMEs: presence lock, subagent
//! listings, the bounded glob, the known-conversation shortcut and the lifecycle reading, then
//! the role and intent `refine` leaves on the decoded recorded payloads.
//! Included from `src/harness/agy/witness.rs`; manifests are `tests/fixtures/agy/layouts/*.txt`.
use super::*;
use crate::harness::{
    adapter::{
        DeliveryEligibility, EventIntent, EventRole, HarnessAdapter, HookInput,
        LocalEvidenceRequest,
    },
    agy::{AgyAdapter, AgyAdmission, InjectionStep},
};
use std::path::PathBuf;
use std::time::Duration;

const TOP: &str = "534e47d6-64b3-4fd6-bd7f-446ceec4d86c";
const CHILD: &str = "5781d6c5-188b-42a5-8f8c-e8b0da366322";
const CLEAR: &str = "e806a19d-b316-4599-b4c5-a56db668fb4a";
const RESUME: &str = "ccac2ead-d847-45fb-aa4b-9fc7126acdc5";

pub(in crate::harness::agy) struct Temp(pub(in crate::harness::agy) PathBuf);
impl Temp {
    pub(in crate::harness::agy) fn new() -> Self {
        let dir = std::env::temp_dir().join(format!("agyw-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/agy")
}
/// Creates every path of `tests/fixtures/agy/layouts/<manifest>.txt` under `home`'s agy data dir.
pub(in crate::harness::agy) fn build_layout(home: &Path, manifest: &str) {
    let data = AgyRoots::from_home(home).data;
    let text = std::fs::read_to_string(fixtures().join("layouts").join(format!("{manifest}.txt")))
        .unwrap();
    for line in text.lines().map(str::trim).filter(|line| !line.is_empty()) {
        if let Some(count) = line.strip_prefix("#generate brain ") {
            let brain = data.join("brain");
            std::fs::create_dir_all(&brain).unwrap();
            for i in 0..count.parse::<usize>().unwrap() {
                std::fs::File::create(brain.join(format!("e{i}"))).unwrap();
            }
        } else if !line.starts_with('#') {
            let path = data.join(line);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::File::create(path).unwrap();
        }
    }
}
fn payload(name: &str) -> Vec<u8> {
    std::fs::read(fixtures().join("payloads").join(format!("{name}.json"))).unwrap()
}
fn input(name: &str, event: &str) -> HookInput {
    HookInput {
        bytes: payload(name),
        registered_event: Some(event.into()),
    }
}

struct Case<'a> {
    home: Option<&'a Path>,
    state: Option<&'a Path>,
    pane: Option<&'a str>,
    deadline: Instant,
}
impl<'a> Case<'a> {
    fn new(home: &'a Path, pane: Option<&'a str>) -> Self {
        Self {
            home: Some(home),
            state: None,
            pane,
            deadline: Instant::now() + Duration::from_secs(30),
        }
    }
    fn evidence(&self, input: &HookInput) -> (LocalEvidence, usize, usize) {
        READ_DIR_CALLS.with(|calls| calls.set(0));
        let pane_asks = std::cell::Cell::new(0);
        let pane_session = || {
            pane_asks.set(pane_asks.get() + 1);
            self.pane.map(str::to_owned)
        };
        let evidence = local_evidence(&LocalEvidenceRequest {
            input,
            home: self.home.map(Path::as_os_str),
            state_dir: self.state,
            pane_session: &pane_session,
            deadline: self.deadline,
        });
        (
            evidence,
            READ_DIR_CALLS.with(|calls| calls.get()),
            pane_asks.get(),
        )
    }
    /// The decoded event after `refine`, as the hook runner leaves it.
    fn refined(&self, input: &HookInput) -> DecodedEvent {
        let mut decoded = AgyAdapter
            .decode(
                &AgyAdmission {
                    step: InjectionStep::SystemMessage,
                },
                input,
            )
            .unwrap();
        self.evidence(input).0.refine(&mut decoded);
        decoded
    }
}
fn role(evidence: &LocalEvidence) -> RoleWitness {
    evidence.role.expect("agy always reports a role witness")
}
fn witnesses(marker: Witness, listing: Witness) -> RoleWitness {
    RoleWitness {
        top_level_marker: marker,
        child_listing: listing,
    }
}
use crate::harness::adapter::DecodedEvent;

/// `EventIntent` has no `PartialEq`: name the two shapes this suite distinguishes.
fn intent(event: &DecodedEvent) -> String {
    match &event.intent {
        EventIntent::Lifecycle(kind) => format!("lifecycle:{kind:?}"),
        EventIntent::Observer => "observer".into(),
        _ => "other".into(),
    }
}

fn home_with(manifest: &str) -> (Temp, PathBuf) {
    let temp = Temp::new();
    let home = temp.0.join("home");
    build_layout(&home, manifest);
    (temp, home)
}

#[test]
fn top_level_startup_is_a_top_level_startup_lifecycle() {
    let (_t, home) = home_with("top-level");
    let case = Case::new(&home, None);
    let input = input("session-start-top", "SessionStart");
    let (evidence, globs, _) = case.evidence(&input);
    assert_eq!(
        role(&evidence),
        witnesses(Witness::Present, Witness::Absent)
    );
    assert_eq!(evidence.lifecycle, Some(EventKind::Startup));
    assert_eq!(globs, 1, "an unknown conversation is checked by the glob");
    let decoded = case.refined(&input);
    assert_eq!(decoded.role, EventRole::TopLevel);
    assert_eq!(intent(&decoded), "lifecycle:Startup");
    assert_eq!(decoded.metadata.context_source, "startup");
    assert!(matches!(decoded.delivery, DeliveryEligibility::Context));
}

#[test]
fn a_child_listed_under_the_bound_parent_is_a_subagent_found_without_a_glob() {
    let (_t, home) = home_with("child-under-bound");
    let case = Case::new(&home, Some(TOP));
    let input = input("session-start-child", "SessionStart");
    let (evidence, globs, _) = case.evidence(&input);
    assert_eq!(
        role(&evidence),
        witnesses(Witness::Present, Witness::Present),
        "a presence lock alone does not make a top-level"
    );
    assert_eq!(
        globs, 0,
        "the bound parent is one stat, tried before any glob"
    );
    let decoded = case.refined(&input);
    assert_eq!(decoded.role, EventRole::Subagent);
    assert_eq!(intent(&decoded), "lifecycle:Startup");
}

#[test]
fn a_child_listed_under_another_parent_is_found_by_the_glob() {
    let (_t, home) = home_with("child-under-other");
    for pane in [None, Some(TOP)] {
        let case = Case::new(&home, pane);
        let input = input("session-start-child", "SessionStart");
        let (evidence, globs, _) = case.evidence(&input);
        assert_eq!(
            role(&evidence),
            witnesses(Witness::Present, Witness::Present),
            "pane {pane:?}"
        );
        assert_eq!(globs, 1);
        assert_eq!(case.refined(&input).role, EventRole::Subagent);
    }
}

#[test]
fn clear_in_a_bound_pane_is_a_clear_lifecycle() {
    let (_t, home) = home_with("clear");
    let case = Case::new(&home, Some(TOP));
    let input = input("session-start-after-clear", "SessionStart");
    let (evidence, _, _) = case.evidence(&input);
    assert_eq!(
        role(&evidence),
        witnesses(Witness::Present, Witness::Absent)
    );
    assert_eq!(evidence.lifecycle, Some(EventKind::Clear));
    let decoded = case.refined(&input);
    assert_eq!(decoded.role, EventRole::TopLevel);
    assert_eq!(intent(&decoded), "lifecycle:Clear");
    assert_eq!(decoded.metadata.context_source, "clear");
}

#[test]
fn inv0_with_many_steps_of_an_unknown_conversation_is_a_resume_even_with_a_stale_lock() {
    let (_t, home) = home_with("resume-stale-lock");
    let input = input("pre-invocation-inv0-resume", "PreInvocation");
    for pane in [None, Some(TOP)] {
        let case = Case::new(&home, pane);
        let (evidence, _, _) = case.evidence(&input);
        assert_eq!(evidence.lifecycle, Some(EventKind::Resume), "pane {pane:?}");
        let decoded = case.refined(&input);
        assert_eq!(decoded.role, EventRole::TopLevel);
        assert_eq!(intent(&decoded), "lifecycle:Resume");
        assert_eq!(
            decoded.metadata.context_source, "resume",
            "only a resume source can reattach a seat (C1)"
        );
        assert_eq!(decoded.metadata.native_event, "PreInvocation");
        assert!(matches!(decoded.delivery, DeliveryEligibility::Context));
    }
    assert_eq!(RESUME, "ccac2ead-d847-45fb-aa4b-9fc7126acdc5");
}

#[test]
fn inv0_with_one_step_is_a_startup_or_a_clear() {
    let (_t, home) = home_with("top-level");
    let input = input("pre-invocation-inv0-fresh", "PreInvocation");
    let startup = Case::new(&home, None);
    assert_eq!(
        startup.evidence(&input).0.lifecycle,
        Some(EventKind::Startup)
    );
    assert_eq!(intent(&startup.refined(&input)), "lifecycle:Startup");
    let other = Case::new(&home, Some(CLEAR));
    assert_eq!(other.evidence(&input).0.lifecycle, Some(EventKind::Clear));
    assert_eq!(intent(&other.refined(&input)), "lifecycle:Clear");
}

#[test]
fn a_known_conversation_runs_no_glob_and_stays_a_top_level_observer() {
    let (t, home) = home_with("top-level");
    let state = t.0.join("state");
    let file = paths::conversation_state(&state, TOP);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, b"{}").unwrap();
    for (name, event) in [
        ("pre-invocation-inv0-fresh", "PreInvocation"),
        ("pre-invocation-inv1", "PreInvocation"),
        ("stop-no-tool-call", "Stop"),
    ] {
        let mut case = Case::new(&home, None);
        case.state = Some(&state);
        let input = input(name, event);
        let (evidence, globs, pane_asks) = case.evidence(&input);
        assert_eq!(globs, 0, "{name}: state file known");
        assert_eq!(
            pane_asks, 0,
            "{name}: the pane scan is skipped when the state file answers"
        );
        assert_eq!(
            role(&evidence),
            witnesses(Witness::Present, Witness::Absent)
        );
        assert_eq!(evidence.lifecycle, None, "{name}");
        let decoded = case.refined(&input);
        assert_eq!(decoded.role, EventRole::TopLevel, "{name}");
        assert_eq!(intent(&decoded), "observer", "{name}");
        assert!(
            matches!(decoded.delivery, DeliveryEligibility::ObserverOnly),
            "{name}"
        );
    }
    // The pane bound to the conversation is the same witness.
    let bound = Case::new(&home, Some(TOP));
    let input = input("pre-invocation-inv1", "PreInvocation");
    let (evidence, globs, _) = bound.evidence(&input);
    assert_eq!(globs, 0, "P == C");
    assert_eq!(evidence.lifecycle, None);
    assert_eq!(intent(&bound.refined(&input)), "observer");
}

#[test]
fn an_unknown_routine_turn_checks_the_listing_and_reads_no_lifecycle() {
    let (_t, home) = home_with("top-level");
    let case = Case::new(&home, None);
    let input = input("pre-invocation-inv1", "PreInvocation");
    let (evidence, globs, _) = case.evidence(&input);
    assert_eq!(globs, 1);
    assert_eq!(evidence.lifecycle, None);
    let decoded = case.refined(&input);
    assert_eq!(decoded.role, EventRole::TopLevel);
    assert_eq!(intent(&decoded), "observer");
}

#[test]
fn an_oversized_or_symlinked_state_file_does_not_make_a_conversation_known() {
    let (t, home) = home_with("top-level");
    let state = t.0.join("state");
    let file = paths::conversation_state(&state, TOP);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    let input = input("pre-invocation-inv1", "PreInvocation");
    let mut case = Case::new(&home, None);
    case.state = Some(&state);
    std::fs::write(&file, vec![b' '; 4097]).unwrap();
    assert_eq!(case.evidence(&input).1, 1, "4097 bytes is past the bound");
    std::fs::write(&file, vec![b' '; 4096]).unwrap();
    assert_eq!(case.evidence(&input).1, 0, "4096 bytes is within it");
    std::fs::remove_file(&file).unwrap();
    let target = t.0.join("target");
    std::fs::write(&target, b"{}").unwrap();
    std::os::unix::fs::symlink(&target, &file).unwrap();
    assert_eq!(
        case.evidence(&input).1,
        1,
        "a symlink is not a regular file"
    );
}

#[test]
fn a_missing_lock_is_a_child_and_reads_nothing_more() {
    let (_t, home) = home_with("missing-lock");
    let case = Case::new(&home, None);
    let input = input("session-start-top", "SessionStart");
    let (evidence, globs, _) = case.evidence(&input);
    assert_eq!(
        role(&evidence),
        witnesses(Witness::Absent, Witness::Unchecked)
    );
    assert_eq!(globs, 0);
    assert_eq!(case.refined(&input).role, EventRole::Subagent);
}

#[test]
fn a_symlinked_lock_is_not_present() {
    let temp = Temp::new();
    let home = temp.0.join("home");
    let presence = AgyRoots::from_home(&home).data.join("presence");
    std::fs::create_dir_all(&presence).unwrap();
    let target = temp.0.join("real.lock");
    std::fs::write(&target, b"").unwrap();
    std::os::unix::fs::symlink(&target, presence.join(format!("{TOP}.lock"))).unwrap();
    let case = Case::new(&home, None);
    let input = input("session-start-top", "SessionStart");
    assert_eq!(
        role(&case.evidence(&input).0),
        witnesses(Witness::Absent, Witness::Unchecked)
    );
    assert_eq!(case.refined(&input).role, EventRole::Subagent);
}

#[test]
fn a_lock_that_cannot_be_examined_is_unreadable() {
    let temp = Temp::new();
    let home = temp.0.join("home");
    let presence = AgyRoots::from_home(&home).data.join("presence");
    std::fs::create_dir_all(&presence).unwrap();
    // A path component that is a file: ENOTDIR, neither a lock nor an absent one.
    std::fs::remove_dir(&presence).unwrap();
    std::fs::write(&presence, b"").unwrap();
    let case = Case::new(&home, None);
    let input = input("session-start-top", "SessionStart");
    assert_eq!(
        role(&case.evidence(&input).0).top_level_marker,
        Witness::Unreadable
    );
    assert_eq!(case.refined(&input).role, EventRole::Subagent);
}

#[test]
fn a_missing_home_is_unreadable_and_reads_as_a_child() {
    let mut case = Case::new(Path::new("/nonexistent"), None);
    case.home = None;
    let input = input("session-start-top", "SessionStart");
    let (evidence, globs, _) = case.evidence(&input);
    assert_eq!(
        role(&evidence),
        witnesses(Witness::Unreadable, Witness::Unreadable)
    );
    assert_eq!(globs, 0);
    assert_eq!(case.refined(&input).role, EventRole::Subagent);
    case.home = Some(Path::new(""));
    assert_eq!(
        role(&case.evidence(&input).0),
        witnesses(Witness::Unreadable, Witness::Unreadable),
        "an empty HOME is a missing one"
    );
}

#[test]
fn a_brain_directory_past_the_cap_reads_as_a_child_and_one_at_the_cap_is_examined() {
    let (_t, home) = home_with("over-cap-brain");
    let case = Case::new(&home, None);
    let input = input("session-start-top", "SessionStart");
    let (evidence, globs, _) = case.evidence(&input);
    assert_eq!(globs, 1);
    assert_eq!(
        role(&evidence),
        witnesses(Witness::Present, Witness::Unreadable)
    );
    assert_eq!(case.refined(&input).role, EventRole::Subagent);
    // One entry fewer is exactly the cap: every entry is examined and none lists the child.
    let brain = AgyRoots::from_home(&home).data.join("brain");
    std::fs::remove_file(brain.join("e0")).unwrap();
    assert_eq!(std::fs::read_dir(&brain).unwrap().count(), BRAIN_ENTRY_CAP);
    assert_eq!(
        role(&case.evidence(&input).0),
        witnesses(Witness::Present, Witness::Absent)
    );
}

#[test]
fn a_passed_deadline_makes_the_glob_unreadable() {
    let (_t, home) = home_with("top-level");
    let mut case = Case::new(&home, None);
    case.deadline = Instant::now();
    let input = input("session-start-top", "SessionStart");
    let (evidence, globs, _) = case.evidence(&input);
    assert_eq!(globs, 0, "the glob is not started past the deadline");
    assert_eq!(
        role(&evidence),
        witnesses(Witness::Present, Witness::Unreadable)
    );
    assert_eq!(case.refined(&input).role, EventRole::Subagent);
}

#[test]
fn an_unusable_request_yields_no_evidence() {
    let (_t, home) = home_with("top-level");
    let case = Case::new(&home, None);
    let bad = |bytes: &[u8], event: Option<&str>| HookInput {
        bytes: bytes.to_vec(),
        registered_event: event.map(str::to_owned),
    };
    for input in [
        bad(&payload("session-start-top"), None),
        bad(&payload("session-start-top"), Some("Other")),
        bad(b"not json", Some("SessionStart")),
        bad(br#"{"conversationId":"../x"}"#, Some("SessionStart")),
        bad(br#"{}"#, Some("SessionStart")),
    ] {
        assert_eq!(case.evidence(&input).0, LocalEvidence::default());
    }
}

#[test]
fn stop_reads_the_role_but_never_a_lifecycle() {
    let (_t, home) = home_with("top-level");
    let case = Case::new(&home, Some(CLEAR));
    let input = input("stop-no-tool-call", "Stop");
    let (evidence, _, _) = case.evidence(&input);
    assert_eq!(evidence.lifecycle, None);
    assert_eq!(intent(&case.refined(&input)), "observer");
    assert_eq!(CHILD.len(), 36);
}
