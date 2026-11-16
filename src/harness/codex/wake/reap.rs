//! Process-owned exact child handles. No queue I/O, metadata callbacks or signals.
use super::control::error;
use crate::protocol::results::ApiError;
use std::{
    process::Child,
    sync::{Mutex, MutexGuard, TryLockError},
};

pub(super) const LIMIT: usize = 32;
pub(super) type Slot<'a> = MutexGuard<'a, Option<Child>>;
pub(super) struct Children {
    slots: [Mutex<Option<Child>>; LIMIT],
}
// Static ownership intentionally outlives an individual node owner. Process exit
// delegates parentage to the OS; node shutdown never waits on these children.
pub(super) static OWNED: Children = Children::new();

fn borrow(slot: &Mutex<Option<Child>>) -> Option<Slot<'_>> {
    match slot.try_lock() {
        Ok(guard) => Some(guard),
        Err(TryLockError::Poisoned(error)) => Some(error.into_inner()),
        Err(TryLockError::WouldBlock) => None,
    }
}
fn poll(slot: &mut Option<Child>) {
    let done = slot.as_mut().is_some_and(|child| match child.try_wait() {
        Ok(Some(_)) => true,
        // Releasing exact-handle bookkeeping is not helper quiescence evidence.
        Err(error) if error.raw_os_error() == Some(libc::ECHILD) => true,
        Ok(None) | Err(_) => false,
    });
    if done {
        *slot = None;
    }
}
impl Children {
    pub(super) const fn new() -> Self {
        Self {
            slots: [const { Mutex::new(None) }; LIMIT],
        }
    }
    /// The guard reserves capacity before spawn and keeps the Child in its slot.
    pub(super) fn reserve(&self) -> Result<Slot<'_>, ApiError> {
        for slot in &self.slots {
            if let Some(mut slot) = borrow(slot) {
                poll(&mut slot);
                if slot.is_none() {
                    return Ok(slot);
                }
            }
        }
        Err(error("queue helper owned-child capacity unavailable"))
    }
    pub(super) fn reap(&self) {
        for slot in &self.slots {
            if let Some(mut slot) = borrow(slot) {
                poll(&mut slot);
            }
        }
    }
    pub(super) fn pending(&self) -> bool {
        self.slots
            .iter()
            .any(|slot| borrow(slot).is_none_or(|slot| slot.is_some()))
    }
}

#[cfg(all(test, feature = "test-support"))]
#[path = "../../../../tests/harness/codex_wake_reap.rs"]
mod tests;
