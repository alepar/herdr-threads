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
