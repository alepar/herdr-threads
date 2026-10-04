//! ht-p03.23: the admission observer re-observes a harness whose resolved
//! binary changed under the running daemon, replaces the slot's observation
//! whole, and logs the change. The clock is fake, so a tick is one `advance`.
use super::*;
use crate::{
    app::{AdmissionReobserver, HarnessObservations},
    daemon::health::HarnessStatus,
    harness::stub_binaries::write_stub_harness,
    protocol::time::{MonoInstant, UtcMillis},
};
use std::sync::atomic::{AtomicI64, AtomicU64, AtomicUsize};
use std::time::Instant;

struct FakeClock {
    mono: AtomicU64,
    utc: AtomicI64,
}
impl Clock for FakeClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(self.utc.load(Ordering::SeqCst))
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(self.mono.load(Ordering::SeqCst))
    }
}

fn wait_until(what: &str, mut condition: impl FnMut() -> bool) {
    let started = Instant::now();
    while !condition() {
        assert!(
            started.elapsed() < Duration::from_secs(20),
            "timed out waiting for {what}"
        );
        thread::sleep(Duration::from_millis(1));
    }
}

/// Kills: an observer that never re-runs admission once a harness was
/// observed (the slot kept the listed claude), a changed binary that is not
/// logged, an unchanged binary that is logged or re-run, and a half-replaced
/// slot.
#[test]
fn swapping_the_binary_reobserves_on_the_next_tick() {
    let dir = std::env::temp_dir().join(format!("ht-reobserve-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    write_stub_harness(&dir, "claude", "2.1.286");
    let lines = Arc::new(Mutex::new(Vec::<String>::new()));
    let sink = Arc::clone(&lines);
    let reobserver = AdmissionReobserver::new(
        Some(dir.clone().into_os_string()),
        Duration::from_secs(5),
        Arc::new(move |line: &str| sink.lock().unwrap().push(line.to_owned())),
    );
    let clock = Arc::new(FakeClock {
        mono: AtomicU64::new(1),
        utc: AtomicI64::new(1_000_000),
    });
    let cancel = Cancellation::default();
    let pacer = Arc::new(Pacer::new(
        Lane::AdmissionObserver.name(),
        clock.clone(),
        cancel.clone(),
    ));
    let slot = Arc::new(Mutex::new(HarnessObservations::default()));
    let passes = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&passes);
    let handle = start_admission_observer(
        move |budget| {
            counted.fetch_add(1, Ordering::SeqCst);
            Ok(reobserver.pass(&budget.cancellation))
        },
        Arc::clone(&slot),
        Arc::clone(&pacer),
        clock.clone(),
        cancel.clone(),
        Arc::new(WorkerStatus::default()),
    )
    .unwrap();
    let advance = |pacer: &Pacer| {
        clock.mono.fetch_add(TICK_MS, Ordering::SeqCst);
        clock.utc.fetch_add(TICK_MS as i64, Ordering::SeqCst);
        pacer.clock_advanced();
    };

    wait_until("the first pass", || pacer.idle_events() >= 1);
    assert!(
        matches!(
            slot.lock().unwrap().status("claude"),
            HarnessStatus::Cooperative {
                live_unverified: false,
                ..
            }
        ),
        "{:?}",
        slot.lock().unwrap().status("claude")
    );

    // Unchanged binary, next tick: no change line.
    advance(&pacer);
    wait_until("the second pass", || pacer.idle_events() >= 2);
    assert!(
        lines.lock().unwrap().is_empty(),
        "{:?}",
        lines.lock().unwrap()
    );

    // Swap the binary for a newer one (a new inode, same size), then tick.
    let staging = std::env::temp_dir().join(format!("ht-reobserve-staging-{}", std::process::id()));
    std::fs::create_dir_all(&staging).unwrap();
    let staged = write_stub_harness(&staging, "claude", "2.1.299");
    std::fs::rename(&staged, dir.join("claude")).unwrap();
    let _ = std::fs::remove_dir_all(&staging);
    advance(&pacer);
    wait_until("the third pass", || pacer.idle_events() >= 3);
    let observed = slot.lock().unwrap().status("claude").clone();
    let HarnessStatus::Optimistic(detail) = &observed else {
        panic!("the swapped binary must be re-observed as optimistic: {observed:?}");
    };
    assert!(detail.starts_with("claude 2.1.299: optimistic"), "{detail}");
    let lines = lines.lock().unwrap();
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(
        lines[0].starts_with("claude binary changed: ")
            && lines[0].ends_with("; admission optimistic")
            && lines[0].contains(" \u{2192} "),
        "{}",
        lines[0]
    );
    assert_eq!(passes.load(Ordering::SeqCst), 3);

    cancel.cancel();
    handle.join().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

const TICK_MS: u64 = ADMISSION_TICK.as_millis() as u64;

/// Kills: an admission pass that ignores the lane's cancellation, so
/// shutdown's join waits out two 5 s --version timeouts (final review S6), a
/// hung harness child left running, and a half-observed pass stored.
#[test]
fn cancelling_the_lane_kills_a_hung_harness_and_ends_the_pass() {
    use std::os::unix::fs::PermissionsExt;
    let dir = std::env::temp_dir().join(format!("ht-reobserve-hung-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let marker = |name: &str| dir.join(format!("{name}.pid"));
    for name in ["claude", "codex"] {
        let path = dir.join(name);
        let pid_file = marker(name);
        std::fs::write(
            &path,
            format!(
                "#!/bin/sh\necho $$ > '{0}.tmp'\nmv '{0}.tmp' '{0}'\nexec sleep 60\n",
                pid_file.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let reobserver = AdmissionReobserver::new(
        Some(dir.clone().into_os_string()),
        Duration::from_secs(30),
        Arc::new(|_: &str| {}),
    );
    let clock = Arc::new(FakeClock {
        mono: AtomicU64::new(1),
        utc: AtomicI64::new(1_000_000),
    });
    let cancel = Cancellation::default();
    let pacer = Arc::new(Pacer::new(
        Lane::AdmissionObserver.name(),
        clock.clone(),
        cancel.clone(),
    ));
    let slot = Arc::new(Mutex::new(HarnessObservations::default()));
    let handle = start_admission_observer(
        move |budget| {
            let observed = reobserver.pass(&budget.cancellation);
            if budget.cancellation.is_cancelled() {
                return Err(ApiError::cancelled("admission pass cancelled"));
            }
            Ok(observed)
        },
        Arc::clone(&slot),
        pacer,
        clock.clone(),
        cancel.clone(),
        Arc::new(WorkerStatus::default()),
    )
    .unwrap();
    wait_until("claude --version started", || marker("claude").exists());
    cancel.cancel();
    handle.join().unwrap();

    assert!(
        !marker("codex").exists(),
        "the pass went on to observe codex after cancellation"
    );
    let pid = std::fs::read_to_string(marker("claude")).unwrap();
    let alive = std::process::Command::new("kill")
        .args(["-0", pid.trim()])
        .status()
        .unwrap()
        .success();
    assert!(!alive, "hung claude child {} outlived the lane", pid.trim());
    let stored = slot.lock().unwrap();
    assert!(
        matches!(stored.status("claude"), HarnessStatus::Unknown)
            && matches!(stored.status("codex"), HarnessStatus::Unknown),
        "a cancelled pass was stored: {:?} / {:?}",
        stored.status("claude"),
        stored.status("codex")
    );
    let _ = std::fs::remove_dir_all(&dir);
}
