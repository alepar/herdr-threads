//! Idle exit for a store-only daemon with nothing to serve (peer-link spec
//! §15 Idle exit). No other idle exit exists: a daemon with a host, a peer
//! link, an enabled dial target or a local client stays up.

use crate::daemon::transport::ConnectionGauge;
use crate::protocol::time::{Cancellation, Clock};
use crate::service::host_sessions::HostSessions;
use crate::sync::coordinator::SyncHandle;
use std::{sync::Arc, time::Duration};

/// How long every liveness input must stay zero before the daemon exits.
pub const IDLE_EXIT_AFTER: Duration = Duration::from_secs(10 * 60);

/// How often the monitor samples the probe in production.
pub const CHECK_EVERY: Duration = Duration::from_secs(30);

/// What keeps the daemon alive: attached hosts, peer links, enabled dial
/// targets and local client connections.
///
/// Responsibility (ActivityProbe)
/// - Owns: the one list of liveness inputs and the "any of them non-zero"
///   rule ([`Self::active`]).
/// - Hides: where each count lives (the registry, the coordinator actor's
///   links and Dialer, the accept loop's gauge).
/// - Must not: hold state, decide when to exit, or count anything itself.
/// - Where new liveness inputs go: a field here and a term in `active`.
/// - Role: the read side of idle exit; a plain struct, not a trait.
pub struct ActivityProbe {
    pub hosts: Arc<HostSessions>,
    pub sync: SyncHandle,
    pub gauge: ConnectionGauge,
}

impl ActivityProbe {
    /// True while any host is attached, any peer link is established or
    /// handshaking (inbound or dialed), any dial target is enabled, or any
    /// local client connection is open.
    pub fn active(&self) -> bool {
        self.hosts.attached_count() != 0
            || self.sync.link_count() != 0
            || self.sync.enabled_target_count() != 0
            || self.gauge.count() != 0
    }
}

/// The pure timer: idle for `period` of continuous inactive observations.
#[derive(Debug, Clone, Copy)]
pub struct IdleTimer {
    period_ms: u64,
    idle_since: Option<u64>,
}

impl IdleTimer {
    /// A zero `period` never fires.
    pub fn new(period: Duration) -> Self {
        Self {
            period_ms: u64::try_from(period.as_millis()).unwrap_or(u64::MAX),
            idle_since: None,
        }
    }

    /// Record one observation at monotonic `now_ms`; true when the daemon has
    /// been inactive for the whole period.
    pub fn observe(&mut self, active: bool, now_ms: u64) -> bool {
        if active || self.period_ms == 0 {
            self.idle_since = None;
            return false;
        }
        let since = *self.idle_since.get_or_insert(now_ms);
        now_ms.saturating_sub(since) >= self.period_ms
    }
}

/// The monitor that ends a store-only daemon with nothing to serve.
///
/// Responsibility (IdleExit)
/// - Owns: sampling the [`ActivityProbe`] every `check_every`, the idle timer,
///   the one log line and cancelling the daemon's `shutdown`, the token the
///   `Stop` path cancels, so the drain is the ordinary one.
/// - Hides: the timer arithmetic ([`IdleTimer`]).
/// - Must not: exit while a host, link, enabled target or client exists, or
///   stop anything itself; a zero period disables it.
/// - Role: the daemon's only idle exit; clients restart a stopped daemon
///   through `ensure_running`.
pub struct IdleExit {
    probe: ActivityProbe,
    clock: Arc<dyn Clock>,
    period: Duration,
    check_every: Duration,
    shutdown: Cancellation,
    log: Arc<dyn Fn(&str) + Send + Sync>,
}

impl IdleExit {
    pub fn new(
        probe: ActivityProbe,
        clock: Arc<dyn Clock>,
        period: Duration,
        shutdown: Cancellation,
        log: Arc<dyn Fn(&str) + Send + Sync>,
    ) -> Self {
        // A short test period is sampled proportionally often.
        let check_every = CHECK_EVERY.min((period / 4).max(Duration::from_millis(20)));
        Self {
            probe,
            clock,
            period,
            check_every,
            shutdown,
            log,
        }
    }

    /// Sample until `stop` is cancelled or the period elapses idle. Returns
    /// true when it cancelled `shutdown`. Blocking: run it on its own thread.
    pub fn run(self, stop: &Cancellation) -> bool {
        if self.period.is_zero() {
            return false;
        }
        let mut timer = IdleTimer::new(self.period);
        while !stop.wait_blocking(self.check_every) {
            let now = self.clock.monotonic_now().0;
            if timer.observe(self.probe.active(), now) {
                (self.log)(&format!(
                    "idle exit: store-only, no peer links, no dial targets, no clients for {} s",
                    self.period.as_secs()
                ));
                self.shutdown.cancel();
                return true;
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PERIOD: Duration = Duration::from_secs(600);

    // Kills: a timer that keeps its start across a non-zero observation.
    #[test]
    fn non_zero_probe_resets_timer() {
        let mut timer = IdleTimer::new(PERIOD);
        assert!(!timer.observe(false, 0));
        assert!(!timer.observe(false, 590_000));
        assert!(!timer.observe(true, 595_000), "activity resets");
        assert!(!timer.observe(false, 600_000), "idle restarts at 600 s");
        assert!(
            !timer.observe(false, 1_199_999),
            "one ms short of a full period"
        );
        assert!(timer.observe(false, 1_200_000));
    }

    // Kills: firing on the first idle sample or before the full period.
    #[test]
    fn exit_only_after_full_period_all_zero() {
        let mut timer = IdleTimer::new(PERIOD);
        for t in (0..600_000).step_by(30_000) {
            assert!(!timer.observe(false, t), "fired early at {t} ms");
        }
        assert!(timer.observe(false, 600_000));
    }

    // Kills: a zero period that fires immediately instead of disabling.
    #[test]
    fn zero_period_disables() {
        let mut timer = IdleTimer::new(Duration::ZERO);
        assert!(!timer.observe(false, 0));
        assert!(!timer.observe(false, u64::MAX));
    }
}
