//! Per-binding hook turn state (N2-turn, spec D21), kept in daemon memory.
//!
//! A harness's turn-start hook records `Busy`; every turn-ending path records `Idle`: a `Stop` that
//! ended the turn, a `StopFailure`, or a stale busy cleared by the screen (`ScreenIdle`, see
//! [`TurnStateBook::observe_screen`]). A `Stop` the daemon answered with a continuation did not end
//! the turn: the harness samples again without a turn-start hook, so the binding stays busy until
//! the `Stop` that actually ends it (`TurnEndDue` answered `due`, then a later `Stop`).
//!
//! A binding the book has not heard from is [`TurnState::Unknown`] and never reads as idle: a daemon
//! restart forgets every binding, and the next hook of each sets it. The wake lane (ht-kqrt.69)
//! types only into `Idle`.
use crate::{harness::adapter::TurnEdge, protocol::ids::SeatId};
use std::{collections::HashMap, sync::Mutex};

/// A busy binding is a candidate for the screen-idle clear only after this long (ms).
pub const STALE_BUSY_MS: u64 = 60_000;
/// The two composer reads that clear a stale busy are at least this far apart (ms).
pub const SCREEN_READ_GAP_MS: u64 = 5_000;
/// Bound on the book: past it the oldest-touched bindings are forgotten (they read `Unknown`).
const MAX_BINDINGS: usize = 4_096;

/// What the daemon knows about a binding's turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnState {
    /// Never reported since the daemon started. Never reads as idle.
    Unknown,
    /// A turn is running (or its end was missed: a missed `Stop` means no wake until the next hook).
    Busy,
    /// The last turn ended.
    Idle,
}

/// A binding's identity for turn state: its seat and binding generation (a rebind starts over).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TurnKey {
    pub seat: SeatId,
    pub generation: u64,
}

#[derive(Debug, Clone, Copy)]
enum Entry {
    Busy {
        since_ms: u64,
        /// The first of the consecutive `Some(false)` screen reads of this busy turn.
        first_clear_ms: Option<u64>,
    },
    Idle,
}

#[derive(Default)]
struct Inner {
    entries: HashMap<TurnKey, (Entry, u64)>,
    touched: u64,
}

/// The daemon's turn state for every binding that reported one.
#[derive(Default)]
pub struct TurnStateBook {
    inner: Mutex<Inner>,
}

impl TurnStateBook {
    fn with<T>(&self, f: impl FnOnce(&mut Inner) -> T) -> T {
        f(&mut self.inner.lock().unwrap_or_else(|e| e.into_inner()))
    }

    fn put(inner: &mut Inner, key: TurnKey, entry: Entry) {
        inner.touched += 1;
        let touched = inner.touched;
        if inner.entries.len() >= MAX_BINDINGS
            && !inner.entries.contains_key(&key)
            && let Some(oldest) = inner
                .entries
                .iter()
                .min_by_key(|(_, (_, at))| *at)
                .map(|(key, _)| key.clone())
        {
            inner.entries.remove(&oldest);
        }
        inner.entries.insert(key, (entry, touched));
    }

    /// The binding's turn state.
    pub fn state(&self, key: &TurnKey) -> TurnState {
        self.with(|inner| match inner.entries.get(key) {
            Some((Entry::Busy { .. }, _)) => TurnState::Busy,
            Some((Entry::Idle, _)) => TurnState::Idle,
            None => TurnState::Unknown,
        })
    }

    /// Record a reported edge; whether it changed the recorded state. `Busy` restarts the busy
    /// clock; `Idle` always records; `ScreenIdle` is accepted only for a binding recorded busy
    /// (an unknown or idle binding has no stale busy to clear).
    pub fn record(&self, key: &TurnKey, edge: TurnEdge, now_ms: u64) -> bool {
        self.with(|inner| match edge {
            TurnEdge::Busy => {
                Self::put(
                    inner,
                    key.clone(),
                    Entry::Busy {
                        since_ms: now_ms,
                        first_clear_ms: None,
                    },
                );
                true
            }
            TurnEdge::Idle => {
                let changed = !matches!(inner.entries.get(key), Some((Entry::Idle, _)));
                Self::put(inner, key.clone(), Entry::Idle);
                changed
            }
            TurnEdge::ScreenIdle => {
                if matches!(inner.entries.get(key), Some((Entry::Busy { .. }, _))) {
                    Self::put(inner, key.clone(), Entry::Idle);
                    true
                } else {
                    false
                }
            }
        })
    }

    /// The daemon's answer to a `Stop` check-in (`TurnEndDue`): `continued` when it owes the turn
    /// a continuation, which makes the harness sample again with no turn-start hook, so the turn
    /// is still running (also when the book had missed its start). Otherwise the turn ends here.
    pub fn answered_turn_end(&self, key: &TurnKey, continued: bool, now_ms: u64) {
        if continued {
            self.with(|inner| {
                if !matches!(inner.entries.get(key), Some((Entry::Busy { .. }, _))) {
                    Self::put(
                        inner,
                        key.clone(),
                        Entry::Busy {
                            since_ms: now_ms,
                            first_clear_ms: None,
                        },
                    );
                }
            });
        } else {
            self.record(key, TurnEdge::Idle, now_ms);
        }
    }

    /// One composer read of a busy binding through `ComposerPolicy::screen_turn_running`. A busy
    /// of at least [`STALE_BUSY_MS`] becomes idle (`TurnEdge::ScreenIdle`) once two reads at least
    /// [`SCREEN_READ_GAP_MS`] apart both answered `Some(false)`; any other answer restarts the
    /// count. Whether this read recorded the idle.
    pub fn observe_screen(&self, key: &TurnKey, running: Option<bool>, now_ms: u64) -> bool {
        let clear = self.with(|inner| {
            let Some((
                Entry::Busy {
                    since_ms,
                    first_clear_ms,
                },
                at,
            )) = inner.entries.get_mut(key)
            else {
                return false;
            };
            if running != Some(false) || now_ms.saturating_sub(*since_ms) < STALE_BUSY_MS {
                *first_clear_ms = None;
                return false;
            }
            let _ = at;
            match *first_clear_ms {
                None => {
                    *first_clear_ms = Some(now_ms);
                    false
                }
                Some(first) => now_ms.saturating_sub(first) >= SCREEN_READ_GAP_MS,
            }
        });
        clear && self.record(key, TurnEdge::ScreenIdle, now_ms)
    }
}
