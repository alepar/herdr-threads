//! Host reachability shared by the observation and wake lanes (ht-72q).
//!
//! The observation lane is the only writer: a capture frozen for
//! unavailability (Herdr not answering, or a capture the daemon could not
//! stage; TRUST-POLICY C4) marks the host down, and any capture Herdr
//! answered marks it up again. While the host is down the wake lane freezes:
//! it reserves nothing, records no refusal and calls no Herdr method. The
//! down-to-up transition kicks the wake lane so it resumes at once instead of
//! at its safety tick.
//!
//! The start state is up (unknown counts as up): the lanes behave as before
//! until the observation lane has positive evidence of an outage.
use crate::service::pacer::Pacer;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Default)]
pub struct HostReachability {
    down: AtomicBool,
    archival: Mutex<ArchivalReachability>,
    /// Down-to-up transitions so far; the wake lane drops its in-memory
    /// refusal backoff when this moves, so its first pass after recovery
    /// attempts every due seat promptly.
    recoveries: AtomicU64,
    /// The wake lane's Pacer, kicked on recovery.
    wake: Mutex<Option<Arc<Pacer>>>,
}

impl HostReachability {
    /// Registers the Pacer the down-to-up transition kicks.
    pub fn attach_wake_pacer(&self, pacer: Arc<Pacer>) {
        if let Ok(mut slot) = self.wake.lock() {
            *slot = Some(pacer);
        }
    }

    /// Whether the last capture was frozen for unavailability.
    pub fn is_down(&self) -> bool {
        self.down.load(Ordering::SeqCst)
    }

    /// The number of down-to-up transitions so far.
    pub fn recoveries(&self) -> u64 {
        self.recoveries.load(Ordering::SeqCst)
    }

    /// A capture was frozen for unavailability.
    pub fn mark_down(&self) {
        self.down.store(true, Ordering::SeqCst);
        self.mark_archival_uncertain();
    }

    /// Herdr answered a capture. On the down-to-up transition this counts a
    /// recovery and kicks the wake lane.
    pub fn mark_up(&self) {
        if self.down.swap(false, Ordering::SeqCst) {
            self.recoveries.fetch_add(1, Ordering::SeqCst);
            let pacer = self.wake.lock().ok().and_then(|slot| slot.clone());
            if let Some(pacer) = pacer {
                pacer.kick();
            }
        }
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct ArchivalReachability {
    pub generation: u64,
    pub coherent: bool,
    pub published_at: Option<u64>,
}
impl HostReachability {
    pub fn archival_state(&self, now: u64) -> ArchivalReachability {
        self.with_archival_state(now, |state| state)
    }
    /// Hold only local evidence synchronization across the short deciding store
    /// transaction. No host or journal I/O is permitted inside this closure.
    pub fn with_archival_state<R>(
        &self,
        now: u64,
        work: impl FnOnce(ArchivalReachability) -> R,
    ) -> R {
        let guard = self.archival.lock();
        let mut state = guard.as_ref().map(|v| **v).unwrap_or_default();
        state.coherent &= state
            .published_at
            .is_some_and(|at| now >= at && now - at <= crate::store::archival::MAX_GAP_MS as u64);
        work(state)
    }
    pub fn mark_archival_published(&self, now: u64) {
        if let Ok(mut state) = self.archival.lock() {
            if !state.coherent
                || state.published_at.is_none_or(|at| {
                    now < at || now - at > crate::store::archival::MAX_GAP_MS as u64
                })
            {
                state.generation = state.generation.saturating_add(1);
            }
            state.coherent = true;
            state.published_at = Some(now);
        }
    }
    pub fn mark_archival_uncertain(&self) {
        if let Ok(mut state) = self.archival.lock() {
            if state.coherent {
                state.generation = state.generation.saturating_add(1);
            }
            state.coherent = false;
        }
    }
}

#[cfg(test)]
mod archival_tests {
    use super::*;
    #[test]
    fn archival_reachability_requires_publication_and_resets_on_every_outage() {
        let state = HostReachability::default();
        assert!(!state.archival_state(0).coherent);
        state.mark_up();
        assert!(
            !state.archival_state(0).coherent,
            "wake's initial-up convention is not archival evidence"
        );
        state.mark_archival_published(10);
        let good = state.archival_state(10);
        assert!(good.coherent);
        assert!(
            !state.archival_state(120_011).coherent,
            "stale capture vetoes even without explicit down"
        );
        state.mark_down();
        let down = state.archival_state(11);
        assert!(!down.coherent);
        assert!(down.generation > good.generation);
        state.mark_up();
        assert!(!state.archival_state(12).coherent);
        state.mark_archival_published(13);
        assert!(state.archival_state(13).coherent);
        assert!(state.archival_state(13).generation > down.generation);
        state.mark_archival_uncertain();
        assert!(!state.archival_state(14).coherent);
    }
}
