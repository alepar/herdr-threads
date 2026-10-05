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
            slot.lock().unwrap().claude,
            HarnessStatus::Cooperative {
                live_unverified: false,
                ..
            }
        ),
        "{:?}",
        slot.lock().unwrap().claude
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
    let observed = slot.lock().unwrap().claude.clone();
    let HarnessStatus::Cooperative { detail, .. } = &observed else {
        panic!("the swapped wrapper must remain cooperative: {observed:?}");
    };
    assert!(detail.contains("contract_declared"), "{detail}");
    assert_eq!(slot.lock().unwrap().claude_version, None);
    let lines = lines.lock().unwrap();
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(
        lines[0].starts_with("claude binary changed: ")
            && lines[0].ends_with("; admission contract_declared")
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

// Catches cancellation replacing a whole existing pair or probing wrappers.
#[test]
fn cancelling_the_lane_preserves_the_previous_observation() {
    let iso = crate::test_support::isolation::TestIsolation::new("task3-observer-cancel");
    write_stub_harness(iso.state_root(), "claude", "2.1.286");
    let observer = AdmissionReobserver::new(
        Some(iso.state_root().as_os_str().to_owned()),
        Duration::from_secs(1),
        Arc::new(|_| {}),
    );
    let previous = observer.pass(&Cancellation::default());
    let cancel = Cancellation::default();
    cancel.cancel();
    assert_eq!(observer.pass(&cancel), HarnessObservations::default());
    assert_eq!(observer.pass(&Cancellation::default()), previous);
}

// Catches any diagnostic execution and accidental rich/native qualification.
#[test]
fn task3_versionless_daemon_observes_failing_wrappers_without_probes() {
    use crate::ports::PokeCapabilitySource;
    use std::os::unix::fs::PermissionsExt;
    let iso = crate::test_support::isolation::TestIsolation::new("task3-observer");
    let dir = iso.state_root();
    let marker = dir.join("invocations");
    for name in ["claude", "codex"] {
        let binary = dir.join(name);
        std::fs::write(
            &binary,
            format!(
                "#!/bin/sh\nprintf '%s\n' \"$*\" >> '{}'\nexit 71\n",
                marker.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(binary, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let observer = AdmissionReobserver::new(
        Some(dir.as_os_str().to_owned()),
        Duration::from_secs(2),
        Arc::new(|_| {}),
    );
    let observed = observer.pass(&Cancellation::default());
    assert!(!marker.exists(), "daemon invoked a diagnostic wrapper");
    for status in [&observed.claude, &observed.codex] {
        assert!(
            matches!(status, HarnessStatus::Cooperative { .. }),
            "{status:?}"
        );
    }
    assert_eq!(observed.claude_version, None);
    assert_eq!(observed.codex_version, None);
    let source = crate::app::ObservedPokeCapabilities::new(Arc::new(Mutex::new(observed)));
    for harness in [
        crate::protocol::authority::Harness::Claude,
        crate::protocol::authority::Harness::Codex,
    ] {
        assert_eq!(
            source.capabilities(harness),
            crate::harness::recipe::PokeCapabilities::NONE
        );
    }
}
