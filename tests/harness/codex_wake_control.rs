use super::*;
use crate::{
    app::SystemClock,
    host::process_table::HostProcessTable,
    protocol::time::{CallBudget, MonoInstant},
};
use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf};
struct Fixture {
    root: PathBuf,
    descriptor: Descriptor,
    clock: SystemClock,
    budget: CallBudget,
}
impl Fixture {
    fn new(mode: &str) -> Self {
        let root = crate::test_support::scratch::temp_path(format!(
            "ht-wake-control-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&root).unwrap();
        let helper = root.join("helper");
        fs::write(&helper, include_str!("../fixtures/codex_queue_helper.py")).unwrap();
        fs::set_permissions(&helper, fs::Permissions::from_mode(0o700)).unwrap();
        fs::write(root.join("queue_1.sqlite"), b"fixture").unwrap();
        fs::write(root.join("other.sqlite"), b"wrong").unwrap();
        fs::write(root.join("mode"), mode).unwrap();
        let lock = root.join("session.lock");
        fs::write(&lock, b"").unwrap();
        let descriptor = Descriptor {
            executable: super::super::capture::identity(&helper).unwrap(),
            database: super::super::capture::identity(&root.join("queue_1.sqlite")).unwrap(),
            session_lock: super::super::capture::identity(&lock).unwrap(),
        };
        Self {
            root,
            descriptor,
            clock: SystemClock::new(),
            budget: CallBudget {
                deadline: MonoInstant(2000),
                cancellation: Default::default(),
            },
        }
    }
    fn context(&self) -> WakeControlContext<'_> {
        WakeControlContext {
            clock: &self.clock,
            budget: &self.budget,
            processes: &HostProcessTable,
            state_dir: &self.root,
            helper_observer: None,
        }
    }
    fn trace(&self) -> Vec<serde_json::Value> {
        fs::read_to_string(self.root.join("trace"))
            .unwrap_or_default()
            .lines()
            .map(|v| serde_json::from_str(v).unwrap())
            .collect()
    }
    fn add(&self, allow: bool) -> WakeAddOutcome {
        enqueue(
            &self.descriptor,
            "00000000-0000-0000-0000-000000000001",
            "owned-attempt-1",
            &mut || Ok(()),
            &mut || Ok(allow),
            &self.context(),
        )
    }
}
/// The process has exited (reaped or a zombie awaiting its parent).
fn exited_or_zombie(pid: u32) -> bool {
    #[cfg(target_os = "linux")]
    {
        fs::read_to_string(format!("/proc/{pid}/stat"))
            .ok()
            .and_then(|s| {
                s.rsplit_once(')')
                    .map(|(_, tail)| tail.trim_start().starts_with('Z'))
            })
            .is_none_or(|zombie| zombie)
    }
    // libproc answers ESRCH for both a reaped pid and a zombie.
    #[cfg(target_os = "macos")]
    {
        super::super::macos::birth(pid) == Err(libc::ESRCH)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
#[test]
fn accepted_add_uses_only_marker_and_private_environment() {
    let f = Fixture::new("ok");
    assert!(
        matches!(f.add(true),WakeAddOutcome::Accepted{id,cleanup_error:None} if id=="queue-owned-1")
    );
    let trace = f.trace();
    let row = trace
        .iter()
        .find(|v| v["method"] == "thread/queue/add")
        .unwrap();
    assert_eq!(
        row["params"]["input"],
        serde_json::json!([{"type":"text","text":"herdr-threads: wake up"}])
    );
    assert!(
        !row["env_keys"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v == "OPENAI_API_KEY" || v == "CODEX_SESSION_ID")
    );
    assert!(
        !trace
            .iter()
            .any(|v| v["method"] == "thread/start" || v["method"] == "thread/resume")
    );
}
#[test]
fn final_callback_refusal_never_writes_add() {
    let f = Fixture::new("ok");
    assert!(matches!(
        f.add(false),
        WakeAddOutcome::RefusedBeforeEffect(_)
    ));
    assert!(f.trace().iter().any(|v| v["method"] == "initialize"));
    assert!(!f.trace().iter().any(|v| v["method"] == "thread/queue/add"));
}
#[test]
fn initialization_failure_is_no_effect() {
    let f = Fixture::new("init_error");
    assert!(matches!(
        f.add(true),
        WakeAddOutcome::RefusedBeforeEffect(_)
    ));
    assert!(!f.trace().iter().any(|v| v["method"] == "thread/queue/add"));
}
#[test]
fn different_opened_database_is_no_effect() {
    let f = Fixture::new("wrong_db");
    assert!(matches!(
        f.add(true),
        WakeAddOutcome::RefusedBeforeEffect(_)
    ));
    assert!(!f.trace().iter().any(|v| v["method"] == "thread/queue/add"));
}
#[test]
fn lost_add_reply_is_uncertain() {
    let f = Fixture::new("lost_reply");
    assert!(matches!(f.add(true), WakeAddOutcome::Unknown(_)));
    assert_eq!(
        f.trace()
            .iter()
            .filter(|v| v["method"] == "thread/queue/add")
            .count(),
        1
    );
}
#[test]
fn oversized_add_response_is_uncertain() {
    let f = Fixture::new("oversize");
    assert!(matches!(f.add(true), WakeAddOutcome::Unknown(_)));
}
#[test]
fn hung_add_is_bounded_and_uncertain() {
    let f = Fixture::new("hang");
    let start = std::time::Instant::now();
    let outcome = f.add(true);
    assert!(
        f.trace().iter().any(|v| v["method"] == "thread/queue/add"),
        "fixture did not reach add: {outcome:?}"
    );
    assert!(
        matches!(&outcome, WakeAddOutcome::Unknown(_)),
        "{outcome:?}"
    );
    assert!(start.elapsed() < std::time::Duration::from_secs(3));
}

#[test]
fn list_only_returns_owned_ids_and_reports_incomplete_pagination() {
    let f = Fixture::new("pages");
    let result = list_entries(
        &f.descriptor,
        "00000000-0000-0000-0000-000000000001",
        "owned-attempt-1",
        &f.context(),
    )
    .unwrap();
    assert_eq!(result.ids, ["owned-0", "owned-1", "owned-2", "owned-3"]);
    assert!(!result.complete);
    assert_eq!(
        f.trace()
            .iter()
            .filter(|v| v["method"] == "thread/queue/list")
            .count(),
        4
    );
}
#[test]
fn delete_reports_absence_without_claiming_cancellation() {
    let f = Fixture::new("absent");
    assert!(
        !delete_entry(
            &f.descriptor,
            "00000000-0000-0000-0000-000000000001",
            "owned-0",
            &f.context()
        )
        .unwrap()
    );
}
#[test]
fn oversized_identity_never_starts_helper() {
    let f = Fixture::new("ok");
    let outcome = enqueue(
        &f.descriptor,
        "00000000-0000-0000-0000-000000000001",
        &"x".repeat(129),
        &mut || Ok(()),
        &mut || Ok(true),
        &f.context(),
    );
    assert!(matches!(outcome, WakeAddOutcome::RefusedBeforeEffect(_)));
    assert!(f.trace().is_empty());
}
#[test]
fn receiver_revalidation_failure_after_initialize_writes_no_add() {
    let f = Fixture::new("ok");
    let mut checks = 0;
    let mut verify = || {
        checks += 1;
        if checks == 2 {
            Err(error("receiver scope ended"))
        } else {
            Ok(())
        }
    };
    let result = enqueue(
        &f.descriptor,
        "00000000-0000-0000-0000-000000000001",
        "owned-attempt-1",
        &mut verify,
        &mut || Ok(true),
        &f.context(),
    );
    assert!(matches!(result, WakeAddOutcome::RefusedBeforeEffect(_)));
    assert!(f.trace().iter().any(|v| v["method"] == "initialize"));
    assert!(!f.trace().iter().any(|v| v["method"] == "thread/queue/add"));
}
#[test]
fn partial_add_write_stays_effect_possible_after_failure() {
    struct Writer(bool);
    impl std::io::Write for Writer {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            if self.0 {
                Err(std::io::Error::from(std::io::ErrorKind::BrokenPipe))
            } else {
                self.0 = true;
                Ok(1)
            }
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut w = Writer(false);
    let mut offset = 0;
    let mut effect = false;
    write_chunk(&mut w, b"request\n", &mut offset, true, &mut effect).unwrap();
    assert_eq!(offset, 1);
    assert!(effect);
    assert!(write_chunk(&mut w, b"request\n", &mut offset, true, &mut effect).is_err());
    assert!(effect);
}
#[test]
fn zero_add_write_is_still_no_effect() {
    struct Writer;
    impl std::io::Write for Writer {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Ok(0)
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut offset = 0;
    let mut effect = false;
    assert!(write_chunk(&mut Writer, b"request\n", &mut offset, true, &mut effect).is_err());
    assert!(!effect);
}
#[test]
fn expired_final_callback_never_writes_add() {
    let f = Fixture::new("ok");
    let result = enqueue(
        &f.descriptor,
        "00000000-0000-0000-0000-000000000001",
        "owned-attempt-1",
        &mut || Ok(()),
        &mut || {
            f.budget.cancellation.cancel();
            Ok(true)
        },
        &f.context(),
    );
    assert!(matches!(result, WakeAddOutcome::RefusedBeforeEffect(_)));
    assert!(!f.trace().iter().any(|v| v["method"] == "thread/queue/add"));
}

#[test]
fn accepted_effect_remains_accepted_while_descendants_are_stopped() {
    let f = Fixture::new("descendant");
    assert!(matches!(f.add(true), WakeAddOutcome::Accepted { .. }));
    let pid = fs::read_to_string(f.root.join("descendant")).unwrap();
    let pid: u32 = pid.trim().parse().unwrap();
    let until = std::time::Instant::now() + std::time::Duration::from_secs(1);
    loop {
        match exited_or_zombie(pid) {
            true => break,
            false => {
                assert!(
                    std::time::Instant::now() < until,
                    "owned descendant still running"
                );
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        }
    }
    assert_eq!(
        fs::read_dir(f.root.join("queue-control")).unwrap().count(),
        0
    );
}

#[test]
fn saved_control_survives_old_receiver_exit_but_new_add_refuses() {
    use crate::{
        harness::wake::{WakeControl, WakeControlTarget, WakeScope},
        protocol::{
            authority::LivenessFingerprint,
            ids::{BindingId, ExecutionId, NativeSessionId},
        },
    };
    let f = Fixture::new("ok");
    fs::remove_file(&f.descriptor.session_lock.path).unwrap();
    let target = WakeControlTarget {
        scope: WakeScope {
            binding: BindingId::new("old-binding"),
            execution: ExecutionId::new("old-execution"),
            native_session: NativeSessionId::new("00000000-0000-0000-0000-000000000001"),
            generation: 1,
            process: LivenessFingerprint { pid: 1, start: 0 },
            kernel_boot: "previous-boot".into(),
        },
        descriptor: serde_json::to_vec(&f.descriptor).unwrap(),
    };
    let control = &super::super::CONTROL;
    let listed = control
        .list(&target, "owned-attempt-1", &f.context())
        .expect("saved queue control does not depend on receiver loadedness");
    assert_eq!(listed.ids, ["owned-0"]);
    assert!(listed.complete);
    assert!(control.delete(&target, "owned-0", &f.context()).unwrap());
    let before = f.trace().len();
    assert!(matches!(
        control.add(
            &target,
            "new-attempt",
            &mut || panic!("dead receiver cannot authorize add"),
            &f.context()
        ),
        WakeAddOutcome::RefusedBeforeEffect(_)
    ));
    assert_eq!(f.trace().len(), before);
}
#[test]
fn cleanup_failure_preserves_accepted_identity_and_is_observable() {
    let result = apply_cleanup(
        WakeAddOutcome::Accepted {
            id: "known-id".into(),
            cleanup_error: None,
        },
        Some(error("cleanup unconfirmed")),
    );
    assert!(matches!(result,WakeAddOutcome::Accepted{id,cleanup_error:Some(_)} if id=="known-id"));
}

#[test]
fn missing_metadata_preflight_refuses_before_authorization_or_add() {
    let f = Fixture::new("missing_metadata");
    let mut called = false;
    let outcome = enqueue(
        &f.descriptor,
        "00000000-0000-0000-0000-000000000001",
        "owned-attempt-1",
        &mut || Ok(()),
        &mut || {
            called = true;
            Ok(true)
        },
        &f.context(),
    );
    assert!(
        matches!(outcome, WakeAddOutcome::RefusedBeforeEffect(_)),
        "{outcome:?}"
    );
    assert!(!called);
    assert!(!f.trace().iter().any(|v| v["method"] == "thread/queue/add"));
}
#[test]
fn successful_preflight_precedes_final_authorization_and_add() {
    let f = Fixture::new("ok");
    let mut called = false;
    let outcome = enqueue(
        &f.descriptor,
        "00000000-0000-0000-0000-000000000001",
        "owned-attempt-1",
        &mut || Ok(()),
        &mut || {
            assert!(
                f.trace()
                    .iter()
                    .any(|v| v["method"] == "thread/queue/list" && v["params"]["limit"] == 1)
            );
            called = true;
            Ok(true)
        },
        &f.context(),
    );
    assert!(called);
    assert!(matches!(outcome, WakeAddOutcome::Accepted { .. }));
}
#[test]
fn malformed_preflight_refuses_before_final_authorization() {
    let f = Fixture::new("malformed_preflight");
    let result = enqueue(
        &f.descriptor,
        "00000000-0000-0000-0000-000000000001",
        "owned-attempt-1",
        &mut || Ok(()),
        &mut || panic!("unqualified target cannot authorize add"),
        &f.context(),
    );
    assert!(matches!(result, WakeAddOutcome::RefusedBeforeEffect(_)));
}
#[test]
fn slow_preflight_with_insufficient_add_budget_writes_no_add() {
    let mut f = Fixture::new("slow_preflight");
    f.budget.deadline = MonoInstant(1000);
    let result = enqueue(
        &f.descriptor,
        "00000000-0000-0000-0000-000000000001",
        "owned-attempt-1",
        &mut || Ok(()),
        &mut || panic!("exhausted preflight cannot authorize add"),
        &f.context(),
    );
    assert!(matches!(result, WakeAddOutcome::RefusedBeforeEffect(_)));
    assert!(!f.trace().iter().any(|v| v["method"] == "thread/queue/add"));
}

#[test]
fn cleanup_status_is_typed_without_changing_primary_failure() {
    let failure = finish_failure(error("request failed"), Some(error("cleanup unconfirmed")));
    assert!(
        failure.cleanup_error.is_some(),
        "cleanup uncertainty must be structural evidence"
    );
    assert_eq!(failure.error.detail, "request failed");
    let failure = finish_failure(error("request failed"), None);
    assert!(failure.cleanup_error.is_none());
}

#[test]
fn every_add_class_preserves_typed_cleanup_separately() {
    for uncertain_cleanup in [false, true] {
        for kind in ["refused", "unknown", "accepted"] {
            let outcome = match kind {
                "refused" => WakeAddOutcome::RefusedBeforeEffect(error("primary").into()),
                "unknown" => WakeAddOutcome::Unknown(error("primary").into()),
                _ => WakeAddOutcome::Accepted {
                    id: "known".into(),
                    cleanup_error: None,
                },
            };
            let result = apply_cleanup(outcome, uncertain_cleanup.then(|| error("cleanup")));
            match result {
                WakeAddOutcome::RefusedBeforeEffect(failure) if kind == "refused" => {
                    assert_eq!(failure.error.detail, "primary");
                    assert_eq!(failure.cleanup_error.is_some(), uncertain_cleanup);
                }
                WakeAddOutcome::Unknown(failure) if kind == "unknown" => {
                    assert_eq!(failure.error.detail, "primary");
                    assert_eq!(failure.cleanup_error.is_some(), uncertain_cleanup);
                }
                WakeAddOutcome::Accepted { id, cleanup_error } if kind == "accepted" => {
                    assert_eq!(id, "known");
                    assert_eq!(cleanup_error.is_some(), uncertain_cleanup);
                }
                other => panic!("effect classification changed: {other:?}"),
            }
        }
    }
}
#[test]
fn list_delete_mapping_keeps_cleanup_evidence_for_success_and_failure() {
    for cleanup in [false, true] {
        let success = finish_read(Ok(false), cleanup.then(|| error("cleanup")));
        if cleanup {
            assert!(success.unwrap_err().cleanup_error.is_some());
        } else {
            assert!(!success.unwrap());
        }
        let failure = finish_read::<bool>(Err(error("primary")), cleanup.then(|| error("cleanup")))
            .unwrap_err();
        assert_eq!(failure.error.detail, "primary");
        assert_eq!(failure.cleanup_error.is_some(), cleanup);
    }
}
#[test]
fn pre_effect_and_unknown_protocol_failures_report_confirmed_cleanup() {
    for (mode, unknown) in [
        ("init_error", false),
        ("missing_metadata", false),
        ("lost_reply", true),
    ] {
        let f = Fixture::new(mode);
        let result = f.add(true);
        let failure = match result {
            WakeAddOutcome::RefusedBeforeEffect(e) if !unknown => e,
            WakeAddOutcome::Unknown(e) if unknown => e,
            other => panic!("{mode}: {other:?}"),
        };
        assert!(failure.cleanup_error.is_none(), "{mode}: {failure:?}");
    }
}

struct Observer {
    reject_start: bool,
    reject_completion: bool,
    witnesses: std::sync::Mutex<Vec<crate::harness::wake::WakeHelperWitness>>,
    completions: std::sync::Mutex<Vec<bool>>,
}
impl crate::harness::wake::WakeHelperObserver for Observer {
    fn started(&self, witness: &crate::harness::wake::WakeHelperWitness) -> Result<(), ApiError> {
        self.witnesses.lock().unwrap().push(witness.clone());
        if self.reject_start {
            Err(error("witness reply unavailable"))
        } else {
            Ok(())
        }
    }
    fn completed(&self, confirmed: bool) -> Result<(), ApiError> {
        self.completions.lock().unwrap().push(confirmed);
        if self.reject_completion {
            Err(error("completion reply unavailable"))
        } else {
            Ok(())
        }
    }
}
impl Observer {
    fn new(reject_start: bool, reject_completion: bool) -> Self {
        Self {
            reject_start,
            reject_completion,
            witnesses: Default::default(),
            completions: Default::default(),
        }
    }
}
#[test]
fn rejected_or_lost_witness_reply_stops_before_initialize() {
    let f = Fixture::new("ok");
    let observer = Observer::new(true, false);
    let mut context = f.context();
    context.helper_observer = Some(&observer);
    let outcome = enqueue(
        &f.descriptor,
        "session",
        "correlation",
        &mut || Ok(()),
        &mut || panic!("uncommitted helper cannot arm"),
        &context,
    );
    assert!(matches!(outcome, WakeAddOutcome::RefusedBeforeEffect(_)));
    assert!(
        f.trace().is_empty(),
        "even initialize requires committed witness"
    );
    let witnesses = observer.witnesses.lock().unwrap();
    assert_eq!(witnesses.len(), 1);
    assert_eq!(witnesses[0].pgid, witnesses[0].process.pid);
    assert!(!witnesses[0].kernel_boot.is_empty());
    assert_eq!(*observer.completions.lock().unwrap(), [true]);
}
#[test]
fn completion_failure_keeps_known_acceptance_and_cleanup_evidence() {
    let f = Fixture::new("ok");
    let observer = Observer::new(false, true);
    let mut context = f.context();
    context.helper_observer = Some(&observer);
    let outcome = enqueue(
        &f.descriptor,
        "session",
        "correlation",
        &mut || Ok(()),
        &mut || Ok(true),
        &context,
    );
    assert!(
        matches!(outcome, WakeAddOutcome::Accepted { id, cleanup_error: Some(_) } if id == "queue-owned-1")
    );
    assert_eq!(*observer.completions.lock().unwrap(), [true]);
}
#[test]
fn all_started_control_paths_record_completion_once() {
    for mode in ["ok", "init_error", "lost_reply"] {
        let f = Fixture::new(mode);
        let observer = Observer::new(false, false);
        let mut context = f.context();
        context.helper_observer = Some(&observer);
        let _ = enqueue(
            &f.descriptor,
            "session",
            "correlation",
            &mut || Ok(()),
            &mut || Ok(true),
            &context,
        );
        assert_eq!(observer.witnesses.lock().unwrap().len(), 1);
        assert_eq!(*observer.completions.lock().unwrap(), [true]);
        let _ = list_entries(&f.descriptor, "session", "correlation", &context);
        let _ = delete_entry(&f.descriptor, "session", "id", &context);
        assert_eq!(observer.witnesses.lock().unwrap().len(), 3);
        assert_eq!(*observer.completions.lock().unwrap(), [true, true, true]);
    }
}

#[test]
fn helper_probe_requires_positive_group_absence_after_pid_reuse() {
    use super::super::helper::{LeaderPresence, classify};
    use crate::harness::wake::{WakeHelperPresence as P, WakeHelperQuiescence as Q};
    for (leader, group, expected) in [
        (LeaderPresence::Same, Some(true), P::Live),
        (LeaderPresence::Same, None, P::Unknown),
        (LeaderPresence::Same, Some(false), P::Unknown),
        (LeaderPresence::Ended, Some(true), P::Unknown),
        (
            LeaderPresence::Ended,
            Some(false),
            P::Quiescent(Q::LeaderEndedAndGroupAbsent),
        ),
        (LeaderPresence::Ended, None, P::Unknown),
        (LeaderPresence::Unknown, Some(false), P::Unknown),
    ] {
        assert_eq!(classify(leader, group), expected);
    }
}
#[test]
fn helper_probe_boot_change_is_quiescent_but_unreadable_leader_is_unknown() {
    use crate::harness::wake::{
        WakeControl, WakeHelperPresence as P, WakeHelperQuiescence as Q, WakeHelperWitness,
    };
    use crate::protocol::authority::LivenessFingerprint;
    let f = Fixture::new("ok");
    let witness = WakeHelperWitness {
        process: LivenessFingerprint {
            pid: std::process::id(),
            start: u64::MAX,
        },
        kernel_boot: "old-kernel".into(),
        pgid: std::process::id(),
    };
    assert_eq!(
        super::super::CONTROL
            .probe_helper(&witness, &f.context())
            .unwrap(),
        P::Quiescent(Q::BootChanged)
    );
    #[cfg(target_os = "linux")]
    {
        let dir = f.root.join("unreadable-proc");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("stat"), "malformed").unwrap();
        assert_eq!(
            super::super::helper::leader_at(&dir, 1),
            super::super::helper::LeaderPresence::Unknown
        );
        fs::remove_dir_all(&dir).unwrap();
        assert_eq!(
            super::super::helper::leader_at(&dir, 1),
            super::super::helper::LeaderPresence::Ended
        );
    }
}

#[test]
fn completion_bookkeeping_failure_preserves_primary_control_class() {
    for mode in ["init_error", "lost_reply"] {
        let f = Fixture::new(mode);
        let observer = Observer::new(false, true);
        let mut context = f.context();
        context.helper_observer = Some(&observer);
        let outcome = enqueue(
            &f.descriptor,
            "session",
            "correlation",
            &mut || Ok(()),
            &mut || Ok(true),
            &context,
        );
        let failure = match outcome {
            WakeAddOutcome::RefusedBeforeEffect(failure) if mode == "init_error" => failure,
            WakeAddOutcome::Unknown(failure) if mode == "lost_reply" => failure,
            other => panic!("primary effect classification lost: {other:?}"),
        };
        assert!(failure.cleanup_error.is_some());
        assert_ne!(failure.error.detail, "completion reply unavailable");
        assert_eq!(*observer.completions.lock().unwrap(), [true]);
    }
}
#[test]
fn helper_probe_distinguishes_live_reused_pid_and_absent_group() {
    use crate::harness::wake::{
        WakeControl, WakeHelperPresence as P, WakeHelperQuiescence as Q, WakeHelperWitness,
    };
    use crate::ports::ProcessInfo;
    use crate::protocol::authority::LivenessFingerprint;
    let f = Fixture::new("ok");
    let mut witness = WakeHelperWitness {
        process: LivenessFingerprint::current_process(),
        kernel_boot: HostProcessTable.kernel_boot().unwrap(),
        pgid: unsafe { libc::getpgrp() } as u32,
    };
    assert_eq!(
        super::super::CONTROL
            .probe_helper(&witness, &f.context())
            .unwrap(),
        P::Live
    );
    witness.process.start += 1;
    assert_eq!(
        super::super::CONTROL
            .probe_helper(&witness, &f.context())
            .unwrap(),
        P::Unknown
    );
    witness.process.pid = i32::MAX as u32;
    witness.pgid = i32::MAX as u32;
    assert_eq!(
        super::super::CONTROL
            .probe_helper(&witness, &f.context())
            .unwrap(),
        P::Quiescent(Q::LeaderEndedAndGroupAbsent)
    );
    #[cfg(target_os = "linux")]
    {
        let dir = f.root.join("unknown-owner");
        fs::create_dir_all(&dir).unwrap();
        fs::copy(
            format!("/proc/{}/stat", std::process::id()),
            dir.join("stat"),
        )
        .unwrap();
        fs::write(dir.join("status"), "Uid: unreadable").unwrap();
        assert_eq!(
            super::super::helper::leader_at(&dir, 1),
            super::super::helper::LeaderPresence::Unknown
        );
    }
    // pgid 1 would broadcast the group probe: never Live.
    witness.process = LivenessFingerprint::current_process();
    witness.pgid = 1;
    assert_eq!(
        super::super::CONTROL
            .probe_helper(&witness, &f.context())
            .unwrap(),
        P::Unknown
    );
}

#[test]
fn replay_cohort_is_derived_without_capture_and_rejects_invalid_session() {
    use crate::{harness::wake::WakeControl, protocol::ids::NativeSessionId};
    let session = NativeSessionId::new("00000000-0000-0000-0000-000000000001");
    let key = super::super::CONTROL
        .replay_scope_for_session(&session)
        .unwrap();
    assert_eq!(key.as_str(), "session:00000000-0000-0000-0000-000000000001");
    assert_eq!(
        super::super::CONTROL
            .replay_scope_for_session(&session)
            .unwrap(),
        key
    );
    assert!(
        super::super::CONTROL
            .replay_scope_for_session(&NativeSessionId::new("invalid"))
            .is_err()
    );
}

#[test]
fn parsed_add_response_with_wrong_or_missing_correlation_or_invalid_id_is_unknown() {
    for mode in [
        "add_wrong_correlation",
        "add_missing_correlation",
        "add_invalid_id",
    ] {
        let f = Fixture::new(mode);
        let outcome = enqueue(
            &f.descriptor,
            "00000000-0000-0000-0000-000000000001",
            "owned-attempt-1",
            &mut || Ok(()),
            &mut || Ok(true),
            &f.context(),
        );
        assert!(
            matches!(outcome, WakeAddOutcome::Unknown(_)),
            "{mode}: {outcome:?}"
        );
        assert_eq!(
            f.trace()
                .iter()
                .filter(|r| r["method"] == "thread/queue/add")
                .count(),
            1
        );
        assert!(
            !f.trace()
                .iter()
                .any(|r| r["method"] == "thread/queue/delete")
        );
    }
}

#[test]
fn deadline_and_early_pipe_failure_keep_child_reap_ownership() {
    use crate::harness::wake::WakeControl;
    if crate::test_support::spawn::ran_in_own_process(
        module_path!(),
        "deadline_and_early_pipe_failure_keep_child_reap_ownership",
    ) {
        return;
    }
    for early_pipe_failure in [false, true] {
        let f = Fixture::new("ok");
        let context = f.context();
        let mut peer = Peer::start(&f.descriptor, &context).unwrap();
        let pid = peer.child_id();
        let witness = super::super::helper::witness(pid).unwrap();
        let failed = if early_pipe_failure {
            peer.finish_with(false, Child::try_wait)
        } else {
            peer.deadline = Instant::now();
            // Model a pending poll followed by scheduler suspension while SIGKILL exits the child.
            peer.finish_with(true, |_| {
                std::thread::sleep(Duration::from_millis(300));
                Ok(None)
            })
        };
        assert!(failed.is_some());
        drop(peer);
        let until = Instant::now() + Duration::from_secs(2);
        while Instant::now() < until {
            if exited_or_zombie(pid) {
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        super::super::CONTROL.reap_owned_helpers();
        let mut status = 0;
        let reaped_here = unsafe { libc::waitpid(pid as i32, &mut status, libc::WNOHANG) };
        assert_eq!(
            reaped_here, -1,
            "child was abandoned unreaped after failure, early_pipe_failure={early_pipe_failure}"
        );
        assert_eq!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::ECHILD)
        );
        assert!(matches!(
            super::super::helper::probe(&witness, &context).unwrap(),
            crate::harness::wake::WakeHelperPresence::Quiescent(_)
        ));
    }
}

#[test]
fn reap_capacity_exhaustion_refuses_before_starting_helper() {
    if crate::test_support::spawn::ran_in_own_process(
        module_path!(),
        "reap_capacity_exhaustion_refuses_before_starting_helper",
    ) {
        return;
    }
    let f = Fixture::new("ok");
    let mut reservations = Vec::new();
    for _ in 0..super::super::reap::LIMIT {
        reservations.push(super::super::reap::OWNED.reserve().unwrap());
    }
    assert!(matches!(
        f.add(true),
        WakeAddOutcome::RefusedBeforeEffect(_)
    ));
    assert!(
        f.trace().is_empty(),
        "capacity admission must precede spawn"
    );
    drop(reservations.pop());
    assert!(matches!(f.add(true), WakeAddOutcome::Accepted { .. }));
    drop(reservations);
    assert!(!super::super::reap::OWNED.pending());
}

#[test]
fn non_utf8_sqlite_parent_is_refused_before_helper_spawn() {
    use std::{ffi::OsString, os::unix::ffi::OsStringExt};
    let mut f = Fixture::new("ok");
    f.descriptor.database.path = f
        .root
        .join(OsString::from_vec(b"invalid-\xff".to_vec()))
        .join("queue_1.sqlite");
    let context = f.context();
    let failure = match Peer::start(&f.descriptor, &context) {
        Ok(_) => panic!("non-UTF-8 storage spawned helper"),
        Err(failure) => failure,
    };
    assert_eq!(failure.error.detail, "invalid queue storage encoding");
    assert!(
        !f.root.join("queue-control").exists(),
        "path refusal must precede scratch/spawn"
    );
    assert!(f.trace().is_empty());
}

#[test]
fn dropping_private_helper_home_preserves_shared_parent() {
    let f = Fixture::new("ok");
    let parent = f.root.join("queue-control");
    let child = parent.join("owned");
    fs::create_dir_all(&child).unwrap();
    drop(Home(child.clone()));
    assert!(!child.exists());
    assert!(
        parent.is_dir(),
        "shared parent must outlive individual helper homes"
    );
}
