use crate::harness::context::*;
use std::{
    fs,
    sync::{Arc, Barrier},
    thread,
    time::Duration,
};
use uuid::Uuid;

fn dir() -> std::path::PathBuf {
    let p = std::path::PathBuf::from("/private/tmp").join(format!("ht-context-{}", Uuid::new_v4()));
    fs::create_dir(&p).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&p, fs::Permissions::from_mode(0o700)).unwrap();
    }
    p
}
fn context(instance: Uuid, execution: Uuid, generation: u64) -> OccupantContext {
    OccupantContext {
        format_version: 1,
        instance,
        seat: "seat".into(),
        target: "pane".into(),
        harness: Harness::Codex,
        binding_generation: generation,
        execution,
        session: SessionReference::Native("same-session".into()),
        role: Role::TopLevel,
    }
}
fn request(instance: Uuid, event: &str, generation: u64) -> PendingCheckIn {
    PendingCheckIn {
        operation_id: Uuid::new_v4(),
        mode: CheckInMode::Lifecycle,
        context: context(instance, Uuid::new_v4(), generation),
        expected_generation: Some(generation),
        event_id: event.into(),
        payload_version: 1,
        payload: b" {\"exact\":true} \n".to_vec(),
    }
}
fn journal(path: &std::path::Path, instance: Uuid) -> ContextJournal {
    ContextJournal::open(path, instance, "seat", Duration::from_millis(300)).unwrap()
}
fn result(p: &PendingCheckIn, generation: u64) -> CheckInResponse {
    let mut c = p.context.clone();
    c.binding_generation = generation;
    CheckInResponse {
        context: c,
        historical: false,
        output: b"offer".to_vec(),
    }
}

#[test]
fn response_loss_retries_frozen_bytes_key_and_execution() {
    let path = dir();
    let instance = Uuid::new_v4();
    let p = request(instance, "start", 0);
    let j = journal(&path, instance);
    j.prepare(p.clone()).unwrap();
    assert!(
        j.dispatch("start", &mut |_p: &PendingCheckIn| Err(
            ContextError::Dispatch("lost".into())
        ))
        .is_err()
    );
    drop(j);
    let j = journal(&path, instance);
    assert_eq!(j.pending().unwrap(), Some(p.clone()));
    let response = j
        .dispatch("start", &mut |actual: &PendingCheckIn| {
            assert_eq!(actual, &p);
            Ok(result(actual, 1))
        })
        .unwrap();
    assert_eq!(j.current().unwrap(), Some(response.context));
    assert_eq!(j.pending().unwrap(), None);
    fs::remove_dir_all(path).unwrap();
}
#[test]
fn duplicate_completed_start_returns_result_without_dispatch() {
    let path = dir();
    let i = Uuid::new_v4();
    let j = journal(&path, i);
    let p = request(i, "start", 0);
    j.prepare(p.clone()).unwrap();
    let r = j
        .dispatch("start", &mut |p: &PendingCheckIn| Ok(result(p, 1)))
        .unwrap();
    j.prepare(p).unwrap();
    assert_eq!(
        j.dispatch("start", &mut |_: &PendingCheckIn| panic!(
            "duplicate publish"
        )),
        Ok(r)
    );
    fs::remove_dir_all(path).unwrap();
}
#[test]
fn resume_rotates_even_same_session_tool_and_compact_reuse() {
    let old = context(Uuid::new_v4(), Uuid::new_v4(), 4);
    for kind in [
        EventKind::Startup,
        EventKind::Restart,
        EventKind::Resume,
        EventKind::Clear,
    ] {
        let c = old.for_event(kind, Uuid::new_v4(), None).unwrap();
        assert_ne!(c.execution, old.execution);
        assert!(matches!(c.session, SessionReference::PluginContext(_)));
    }
    let resumed = old
        .for_event(
            EventKind::Resume,
            Uuid::new_v4(),
            Some("same-session".into()),
        )
        .unwrap();
    assert_eq!(resumed.session, old.session);
    assert_ne!(resumed.execution, old.execution);
    for kind in [EventKind::Tool, EventKind::Compact] {
        assert_eq!(old.for_event(kind, Uuid::new_v4(), None).unwrap(), old);
    }
}
#[test]
fn stale_rejection_keeps_pending_and_cannot_rebase() {
    let path = dir();
    let i = Uuid::new_v4();
    let j = journal(&path, i);
    let p = request(i, "start", 0);
    j.prepare(p.clone()).unwrap();
    assert_eq!(
        j.dispatch("start", &mut |_: &PendingCheckIn| Err(
            ContextError::Conflict
        )),
        Err(ContextError::Conflict)
    );
    let mut changed = p.clone();
    changed.expected_generation = Some(9);
    assert_eq!(j.prepare(changed), Err(ContextError::Conflict));
    assert_eq!(j.pending().unwrap(), Some(p));
    assert_eq!(j.current().unwrap(), None);
    fs::remove_dir_all(path).unwrap();
}
#[test]
fn historical_completion_cannot_roll_back_current() {
    let path = dir();
    let i = Uuid::new_v4();
    let j = journal(&path, i);
    let p = request(i, "start", 0);
    j.prepare(p).unwrap();
    j.dispatch("start", &mut |p: &PendingCheckIn| Ok(result(p, 1)))
        .unwrap();
    let p = request(i, "resume", 1);
    j.prepare(p).unwrap();
    let latest = j
        .dispatch("resume", &mut |p: &PendingCheckIn| Ok(result(p, 2)))
        .unwrap();
    let p = request(i, "old-replay", 2);
    j.prepare(p).unwrap();
    j.dispatch("old-replay", &mut |p: &PendingCheckIn| {
        let mut r = result(p, 1);
        r.historical = true;
        Ok(r)
    })
    .unwrap();
    assert_eq!(j.current().unwrap(), Some(latest.context));
    fs::remove_dir_all(path).unwrap();
}
#[test]
fn cross_instance_missing_corrupt_child_and_oversize_are_explicit() {
    let path = dir();
    let i = Uuid::new_v4();
    let j = journal(&path, i);
    assert_eq!(j.current(), Ok(None));
    let mut p = request(i, "tool", 0);
    p.mode = CheckInMode::Current;
    p.expected_generation = None;
    assert_eq!(j.prepare(p), Err(ContextError::LifecycleRequired));
    let mut p = request(i, "child", 0);
    p.context.role = Role::Subagent;
    assert_eq!(j.prepare(p), Err(ContextError::Child));
    let mut p = request(i, "huge", 0);
    p.payload = vec![0; 65537];
    assert_eq!(j.prepare(p), Err(ContextError::TooLarge));
    let p = request(i, "start", 0);
    j.prepare(p).unwrap();
    assert_eq!(
        journal(&path, Uuid::new_v4()).pending(),
        Err(ContextError::WrongInstance)
    );
    fs::write(path.join("context.json"), b"{").unwrap();
    assert_eq!(j.current(), Err(ContextError::Corrupt));
    fs::remove_dir_all(path).unwrap();
}
#[test]
fn concurrent_hooks_publish_once() {
    let path = dir();
    let i = Uuid::new_v4();
    let j = journal(&path, i);
    j.prepare(request(i, "start", 0)).unwrap();
    let barrier = Arc::new(Barrier::new(2));
    let count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let handles: Vec<_> = (0..2)
        .map(|_| {
            let path = path.clone();
            let b = barrier.clone();
            let c = count.clone();
            thread::spawn(move || {
                b.wait();
                journal(&path, i)
                    .dispatch("start", &mut |p: &PendingCheckIn| {
                        c.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        thread::sleep(Duration::from_millis(25));
                        Ok(result(p, 1))
                    })
                    .unwrap()
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }
    assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 1);
    fs::remove_dir_all(path).unwrap();
}
#[cfg(unix)]
#[test]
fn symlink_state_and_permissive_directory_are_refused() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let path = dir();
    let i = Uuid::new_v4();
    symlink("/dev/null", path.join("context.json")).unwrap();
    assert_eq!(journal(&path, i).current(), Err(ContextError::UnsafePath));
    fs::remove_file(path.join("context.json")).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(matches!(
        ContextJournal::open(&path, i, "seat", Duration::from_millis(5)),
        Err(ContextError::UnsafePath)
    ));
    fs::remove_dir_all(path).unwrap();
}
#[cfg(unix)]
#[test]
fn directory_symlink_is_not_hidden_by_canonicalization() {
    use std::os::unix::fs::symlink;
    let path = dir();
    let link = path.with_extension("link");
    symlink(&path, &link).unwrap();
    assert!(matches!(
        ContextJournal::open(&link, Uuid::new_v4(), "seat", Duration::from_millis(5)),
        Err(ContextError::UnsafePath)
    ));
    fs::remove_file(link).unwrap();
    fs::remove_dir_all(path).unwrap();
}
#[test]
fn response_validation_failure_preserves_pre_dispatch_checkpoint() {
    let path = dir();
    let i = Uuid::new_v4();
    let j = journal(&path, i);
    let p = request(i, "start", 0);
    j.prepare(p.clone()).unwrap();
    assert_eq!(
        j.dispatch("start", &mut |p: &PendingCheckIn| {
            let mut r = result(p, 1);
            r.context.execution = Uuid::new_v4();
            Ok(r)
        }),
        Err(ContextError::Conflict)
    );
    assert_eq!(j.pending().unwrap(), Some(p));
    assert_eq!(j.current().unwrap(), None);
    fs::remove_dir_all(path).unwrap();
}
#[test]
fn lock_wait_is_bounded_and_crashed_lock_owner_releases() {
    let path = dir();
    let i = Uuid::new_v4();
    let j = journal(&path, i);
    let _ = j.current().unwrap();
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path.join("context.lock"))
        .unwrap();
    file.lock().unwrap();
    assert_eq!(
        ContextJournal::open(&path, i, "seat", Duration::from_millis(5))
            .unwrap()
            .current(),
        Err(ContextError::LockTimeout)
    );
    drop(file);
    assert_eq!(j.current(), Ok(None));
    fs::remove_dir_all(path).unwrap();
}
#[cfg(unix)]
#[test]
fn crash_after_response_before_completion_leaves_retryable_pending() {
    use std::os::unix::fs::PermissionsExt;
    let path = dir();
    let i = Uuid::new_v4();
    let j = journal(&path, i);
    let p = request(i, "start", 0);
    j.prepare(p.clone()).unwrap();
    let attempt = j.dispatch("start", &mut |p: &PendingCheckIn| {
        fs::set_permissions(&path, fs::Permissions::from_mode(0o500)).unwrap();
        Ok(result(p, 1))
    });
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    assert!(matches!(attempt, Err(ContextError::Io(_))));
    assert_eq!(j.pending().unwrap(), Some(p.clone()));
    assert_eq!(j.current().unwrap(), None);
    j.dispatch("start", &mut |actual: &PendingCheckIn| {
        assert_eq!(actual, &p);
        Ok(result(actual, 1))
    })
    .unwrap();
    fs::remove_dir_all(path).unwrap();
}
#[test]
fn current_mode_after_outage_keeps_complete_context() {
    let path = dir();
    let i = Uuid::new_v4();
    let j = journal(&path, i);
    j.prepare(request(i, "start", 0)).unwrap();
    let registered = j
        .dispatch("start", &mut |p: &PendingCheckIn| Ok(result(p, 1)))
        .unwrap();
    let p = PendingCheckIn {
        operation_id: Uuid::new_v4(),
        mode: CheckInMode::Current,
        context: registered.context.clone(),
        expected_generation: None,
        event_id: "tool".into(),
        payload_version: 1,
        payload: b"current exact bytes".to_vec(),
    };
    j.prepare(p.clone()).unwrap();
    assert!(
        j.dispatch("tool", &mut |_: &PendingCheckIn| Err(
            ContextError::Dispatch("outage".into())
        ))
        .is_err()
    );
    assert_eq!(j.current().unwrap(), Some(registered.context.clone()));
    j.dispatch("tool", &mut |actual: &PendingCheckIn| {
        assert_eq!(actual, &p);
        Ok(CheckInResponse {
            context: actual.context.clone(),
            historical: false,
            output: vec![],
        })
    })
    .unwrap();
    assert_eq!(j.current().unwrap(), Some(registered.context));
    fs::remove_dir_all(path).unwrap();
}
#[test]
fn prepare_reserves_completion_space_before_dispatch() {
    let path = dir();
    let i = Uuid::new_v4();
    let j = journal(&path, i);
    for generation in 0..3 {
        let id = format!("event-{generation}");
        j.prepare(request(i, &id, generation)).unwrap();
        j.dispatch(&id, &mut |p: &PendingCheckIn| {
            let mut r = result(p, generation + 1);
            r.output = vec![255; 65536];
            Ok(r)
        })
        .unwrap();
    }
    // Size retention (kills "reserve without pruning", which wedged the seat
    // with TooLarge): the oldest completions make room for the reservation.
    j.prepare(request(i, "would-overflow", 3)).unwrap();
    assert!(j.pending().unwrap().is_some());
    assert!(j.completed_for_event("event-0").unwrap().is_none());
    assert!(j.completed_for_event("event-2").unwrap().is_some());
    fs::remove_dir_all(path).unwrap();
}

#[test]
fn request_lookup_recovers_exact_completed_request_without_rebasing() {
    let path = dir();
    let instance = Uuid::new_v4();
    let j = journal(&path, instance);
    assert_eq!(j.request_for_event("start").unwrap(), None);
    let first = request(instance, "start", 0);
    j.prepare(first.clone()).unwrap();
    assert_eq!(j.request_for_event("start").unwrap(), Some(first.clone()));
    j.dispatch("start", &mut |p: &PendingCheckIn| Ok(result(p, 1)))
        .unwrap();
    j.prepare(request(instance, "resume", 1)).unwrap();
    let latest = j
        .dispatch("resume", &mut |p: &PendingCheckIn| Ok(result(p, 2)))
        .unwrap();
    drop(j);
    let j = journal(&path, instance);
    assert_eq!(j.request_for_event("start").unwrap(), Some(first.clone()));
    assert_eq!(
        j.get_or_prepare("start", |_| panic!("completed request factory ran"))
            .unwrap(),
        first
    );
    assert_eq!(j.current().unwrap(), Some(latest.context));
    fs::remove_dir_all(path).unwrap();
}

#[test]
fn atomic_get_or_prepare_runs_one_external_intent_factory_for_concurrent_duplicates() {
    use std::io::Write;
    let path = dir();
    let instance = Uuid::new_v4();
    let barrier = Arc::new(Barrier::new(2));
    let handles: Vec<_> = (0..2)
        .map(|_| {
            let path = path.clone();
            let barrier = barrier.clone();
            thread::spawn(move || {
                barrier.wait();
                // The loser waits on the context lock while the winner fsyncs
                // (F_FULLFSYNC on macOS) and sleeps. The default 300ms test
                // budget made that wait a wall-clock race under IO load; use
                // the maximum budget so only a real deadlock can time out.
                ContextJournal::open(&path, instance, "seat", Duration::from_secs(10))
                    .unwrap()
                    .get_or_prepare("start", |current| {
                        assert_eq!(current, None);
                        let frozen = request(instance, "start", 0);
                        // Model the external intent journal's real durable key allocation.
                        let mut file = fs::OpenOptions::new()
                            .create(true)
                            .append(true)
                            .open(path.join("external-intent-keys"))
                            .unwrap();
                        writeln!(file, "{}", frozen.operation_id).unwrap();
                        file.sync_all().unwrap();
                        thread::sleep(Duration::from_millis(25));
                        Ok(frozen)
                    })
                    .unwrap()
            })
        })
        .collect();
    let frozen: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    assert_eq!(frozen[0], frozen[1]);
    assert_eq!(
        fs::read_to_string(path.join("external-intent-keys")).unwrap(),
        format!("{}\n", frozen[0].operation_id)
    );
    assert_eq!(
        journal(&path, instance).pending().unwrap(),
        Some(frozen[0].clone())
    );
    fs::remove_dir_all(path).unwrap();
}

#[test]
fn get_or_prepare_conflict_and_factory_error_do_not_publish_or_rebase() {
    let path = dir();
    let instance = Uuid::new_v4();
    let j = journal(&path, instance);
    assert_eq!(
        j.get_or_prepare("start", |_| Err(ContextError::Dispatch(
            "intent unavailable".into()
        ))),
        Err(ContextError::Dispatch("intent unavailable".into()))
    );
    assert_eq!(j.pending().unwrap(), None);
    let first = j
        .get_or_prepare("start", |_| Ok(request(instance, "start", 0)))
        .unwrap();
    assert_eq!(
        j.get_or_prepare("resume", |_| panic!("conflicting event factory ran")),
        Err(ContextError::Conflict)
    );
    assert_eq!(j.pending().unwrap(), Some(first));
    assert_eq!(
        j.get_or_prepare("", |_| panic!("invalid event factory ran")),
        Err(ContextError::Invalid)
    );
    fs::remove_dir_all(path).unwrap();
}

#[test]
fn fresh_factory_receives_current_context_and_must_return_matching_event() {
    let path = dir();
    let instance = Uuid::new_v4();
    let j = journal(&path, instance);
    j.prepare(request(instance, "start", 0)).unwrap();
    let current = j
        .dispatch("start", &mut |p: &PendingCheckIn| Ok(result(p, 1)))
        .unwrap()
        .context;
    assert_eq!(
        j.get_or_prepare("resume", |_| Ok(request(instance, "wrong-event", 1))),
        Err(ContextError::Conflict)
    );
    assert_eq!(j.pending().unwrap(), None);
    let frozen = j
        .get_or_prepare("resume", |context| {
            assert_eq!(context, Some(&current));
            Ok(request(instance, "resume", 1))
        })
        .unwrap();
    assert_eq!(j.pending().unwrap(), Some(frozen));
    fs::remove_dir_all(path).unwrap();
}

fn lifecycle_cycle(j: &ContextJournal, instance: Uuid, event: &str) -> CheckInResponse {
    let current = j.current().unwrap();
    let generation = current.as_ref().map_or(0, |c| c.binding_generation);
    let mut p = request(instance, event, generation);
    if current.is_none() {
        p.expected_generation = Some(0);
    }
    j.prepare(p).unwrap();
    j.dispatch(event, &mut |p: &PendingCheckIn| {
        Ok(result(p, generation + 1))
    })
    .unwrap()
}

// Kills: "no pruning" (completed history grows to MAX_HISTORY and every later
// prepare returns TooLarge, wedging the seat after about 128 check-ins).
#[test]
fn completed_history_is_bounded_and_never_wedges() {
    let path = dir();
    let i = Uuid::new_v4();
    let j = journal(&path, i);
    for n in 0..200 {
        lifecycle_cycle(&j, i, &format!("event-{n}"));
    }
    assert_eq!(j.completed_len().unwrap(), RETAIN_COMPLETED);
    // Only the newest entries remain replayable; `current` stays authoritative.
    assert!(j.completed_for_event("event-0").unwrap().is_none());
    assert!(j.completed_for_event("event-199").unwrap().is_some());
    assert_eq!(j.current().unwrap().unwrap().binding_generation, 200);
    fs::remove_dir_all(path).unwrap();
}

// Kills: pruning by count only (entries older than the age bound survive).
#[test]
fn completed_history_is_pruned_by_age() {
    let path = dir();
    let i = Uuid::new_v4();
    let j = journal(&path, i);
    for n in 0..3 {
        lifecycle_cycle(&j, i, &format!("old-{n}"));
    }
    let file = path.join("context.json");
    let mut state: serde_json::Value = serde_json::from_slice(&fs::read(&file).unwrap()).unwrap();
    for done in state["completed"].as_array_mut().unwrap() {
        done["completed_at_millis"] = serde_json::json!(1);
    }
    fs::write(&file, serde_json::to_vec(&state).unwrap()).unwrap();
    lifecycle_cycle(&j, i, "fresh");
    assert_eq!(j.completed_len().unwrap(), 1);
    assert!(j.completed_for_event("old-2").unwrap().is_none());
    assert!(j.completed_for_event("fresh").unwrap().is_some());
    fs::remove_dir_all(path).unwrap();
}

// Kills: "never clear pending" and an abandon that does not record the
// rejected key as terminal (the dead request could be re-prepared).
#[test]
fn definitive_rejection_abandons_pending_so_a_fresh_event_prepares() {
    let path = dir();
    let i = Uuid::new_v4();
    let j = journal(&path, i);
    let dead = request(i, "rejected", 0);
    j.prepare(dead.clone()).unwrap();
    assert!(
        j.dispatch("rejected", &mut |_p: &PendingCheckIn| Err(
            ContextError::Dispatch("Conflict".into())
        ))
        .is_err()
    );
    // While pending, an unrelated lifecycle event conflicts.
    assert_eq!(
        j.prepare(request(i, "blocked", 0)),
        Err(ContextError::Conflict)
    );
    assert_eq!(j.abandon_pending("other"), Ok(None));
    assert_eq!(j.abandon_pending("rejected"), Ok(Some(dead.clone())));
    assert_eq!(j.pending().unwrap(), None);
    assert_eq!(j.prepare(dead.clone()), Err(ContextError::Conflict));
    assert_eq!(
        j.get_or_prepare("rejected", |_| Ok(dead.clone())),
        Err(ContextError::Conflict)
    );
    let fresh = request(i, "fresh", 2);
    j.prepare(fresh).unwrap();
    j.dispatch("fresh", &mut |p: &PendingCheckIn| Ok(result(p, 3)))
        .unwrap();
    assert_eq!(j.current().unwrap().unwrap().binding_generation, 3);
    fs::remove_dir_all(path).unwrap();
}

// Kills: a retire that clears a current context it was not shown, or one that
// runs while a request is pending.
#[test]
fn stale_current_is_retired_only_when_unchanged_and_idle() {
    let path = dir();
    let i = Uuid::new_v4();
    let j = journal(&path, i);
    lifecycle_cycle(&j, i, "start");
    let saved = j.current().unwrap().unwrap();
    let mut other = saved.clone();
    other.execution = Uuid::new_v4();
    assert_eq!(j.retire_current(&other), Ok(false));
    j.prepare(request(i, "pending", saved.binding_generation))
        .unwrap();
    assert_eq!(j.retire_current(&saved), Ok(false));
    assert!(j.abandon_pending("pending").unwrap().is_some());
    assert_eq!(j.retire_current(&saved), Ok(true));
    assert_eq!(j.current().unwrap(), None);
    fs::remove_dir_all(path).unwrap();
}

// Kills: an attention mark shared across executions (a new execution would
// inherit the old frontier and suppress its first offer).
#[test]
fn attention_mark_is_per_execution_last_writer_wins() {
    let path = dir();
    let i = Uuid::new_v4();
    let j = journal(&path, i);
    let a = Uuid::new_v4();
    let b = Uuid::new_v4();
    assert_eq!(j.attention_mark(a), None);
    let items = crate::protocol::attention::AttentionToken {
        receipt: Some((7, 0)),
        unavailability_episode: 1,
        ..Default::default()
    };
    j.set_attention_mark(a, &items).unwrap();
    assert_eq!(j.attention_mark(a), Some(items));
    assert_eq!(j.attention_mark(b), None);
    j.set_attention_mark(b, &Default::default()).unwrap();
    assert_eq!(j.attention_mark(a), None);
    assert_eq!(j.attention_mark(b), Some(Default::default()));
    // A retired version-1 frontier file reads as absent, never as a mark.
    fs::write(
        path.join("attention.json"),
        format!(r#"{{"version":1,"execution":"{b}","items":["r:m1"]}}"#),
    )
    .unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(
            path.join("attention.json"),
            fs::Permissions::from_mode(0o600),
        )
        .unwrap();
    }
    assert_eq!(j.attention_mark(b), None);
    // The current shape under the retired version number is rejected by the
    // version gate itself.
    fs::write(
        path.join("attention.json"),
        format!(r#"{{"version":1,"execution":"{b}","token":"v1._.7-0._.1"}}"#),
    )
    .unwrap();
    assert_eq!(j.attention_mark(b), None);
    // The same file at the current version is read (the rejections above are
    // the version gate and the shape, not the file's metadata).
    fs::write(
        path.join("attention.json"),
        format!(r#"{{"version":2,"execution":"{b}","token":"v1._.7-0._.1"}}"#),
    )
    .unwrap();
    assert_eq!(j.attention_mark(b), Some(items));
    // The mark is not the journal: pending/current/completed are untouched.
    assert_eq!(j.pending().unwrap(), None);
    assert_eq!(j.completed_len().unwrap(), 0);
    fs::remove_dir_all(path).unwrap();
}

// Kills: a prepare that refuses a person's lifecycle request over an agent's
// current context (me init --operator unusable), and a dispatch failure that
// clears or replaces the agent's context before the daemon accepted the
// check-in.
#[test]
fn person_lifecycle_request_over_agent_context_keeps_current_until_dispatch() {
    let path = dir();
    let i = Uuid::new_v4();
    let j = journal(&path, i);
    j.prepare(request(i, "agent", 0)).unwrap();
    let agent = j
        .dispatch("agent", &mut |p: &PendingCheckIn| Ok(result(p, 1)))
        .unwrap()
        .context;
    assert_eq!(agent.harness, Harness::Codex);

    let mut person = request(i, "person", 1);
    person.context.harness = Harness::Human;
    person.context.session = SessionReference::PluginContext(person.context.execution);
    j.prepare(person.clone()).unwrap();
    // The daemon rejects: the agent's context is untouched.
    assert!(
        j.dispatch("person", &mut |_p: &PendingCheckIn| Err(
            ContextError::Dispatch("Conflict".into())
        ))
        .is_err()
    );
    assert_eq!(j.current().unwrap(), Some(agent.clone()));
    // The daemon accepts: the human context replaces it.
    let accepted = j
        .dispatch("person", &mut |p: &PendingCheckIn| Ok(result(p, 2)))
        .unwrap()
        .context;
    assert_eq!(accepted.harness, Harness::Human);
    assert_eq!(j.current().unwrap(), Some(accepted));
    fs::remove_dir_all(path).unwrap();
}

// Kills: an agent request over a person, or a different target, newly
// admitted by the person-over-agent allowance.
#[test]
fn lifecycle_harness_change_is_admitted_only_for_a_person_over_an_agent() {
    let path = dir();
    let i = Uuid::new_v4();
    let j = journal(&path, i);
    j.prepare(request(i, "agent", 0)).unwrap();
    j.dispatch("agent", &mut |p: &PendingCheckIn| Ok(result(p, 1)))
        .unwrap();
    let mut claude = request(i, "claude", 1);
    claude.context.harness = Harness::Claude;
    assert_eq!(j.prepare(claude), Err(ContextError::Conflict));
    let mut elsewhere = request(i, "elsewhere", 1);
    elsewhere.context.harness = Harness::Human;
    elsewhere.context.session = SessionReference::PluginContext(elsewhere.context.execution);
    elsewhere.context.target = "other-pane".into();
    assert_eq!(j.prepare(elsewhere), Err(ContextError::Conflict));
    fs::remove_dir_all(path).unwrap();
}

// Kills: an operator mark that applies to any execution, or one that does not
// survive a reopen.
#[test]
fn operator_mark_round_trips_and_is_execution_scoped() {
    let path = dir();
    let i = Uuid::new_v4();
    let j = journal(&path, i);
    let (a, b) = (Uuid::new_v4(), Uuid::new_v4());
    assert_eq!(j.operator_mark(), None);
    assert_eq!(j.set_operator_mark(Uuid::nil()), Err(ContextError::Invalid));
    j.set_operator_mark(a).unwrap();
    assert_eq!(j.operator_mark(), Some(a));
    assert_ne!(j.operator_mark(), Some(b));
    drop(j);
    let j = journal(&path, i);
    assert_eq!(j.operator_mark(), Some(a));
    j.set_operator_mark(b).unwrap();
    assert_eq!(j.operator_mark(), Some(b));
    fs::remove_dir_all(path).unwrap();
}
