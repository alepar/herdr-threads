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
    let stat = PathBuf::from(format!("/proc/{}/stat", pid.trim()));
    let until = std::time::Instant::now() + std::time::Duration::from_secs(1);
    loop {
        match fs::read_to_string(&stat) {
            Err(_) => break,
            Ok(s) if s.rsplit_once(')').unwrap().1.split_whitespace().next() == Some("Z") => break,
            _ => {
                assert!(
                    std::time::Instant::now() < until,
                    "owned descendant still running"
                );
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        }
    }
    assert!(!f.root.join("queue-control").exists());
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
