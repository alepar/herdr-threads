//! A test-only pause inside a real SQLite search transaction.
use crate::protocol::time::{CallBudget, Clock};
use std::{
    collections::HashMap,
    sync::{Arc, Condvar, Mutex, OnceLock},
    time::{Duration, Instant},
};

#[derive(Default)]
struct State {
    entered: usize,
    admitted: usize,
    released: bool,
}
struct Gate {
    state: Mutex<State>,
    changed: Condvar,
    release_on: ReleaseOn,
}
#[derive(Clone, Copy)]
enum ReleaseOn {
    Budget,
    Cancellation,
    Explicit,
}
fn registry() -> &'static Mutex<HashMap<String, Arc<Gate>>> {
    static REGISTRY: OnceLock<Mutex<HashMap<String, Arc<Gate>>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Each fixture uses a unique literal, so parallel tests cannot share a gate.
pub struct SearchBarrier {
    literal: String,
    gate: Arc<Gate>,
}
impl SearchBarrier {
    pub fn install(literal: impl Into<String>) -> Self {
        Self::install_with_release(literal, ReleaseOn::Budget)
    }
    /// Model a read that exits only when its server cancellation reaches it.
    pub fn install_until_cancelled(literal: impl Into<String>) -> Self {
        Self::install_with_release(literal, ReleaseOn::Cancellation)
    }
    /// Simulate a test-controlled SQLite syscall that cannot observe cancellation.
    pub fn install_stalled(literal: impl Into<String>) -> Self {
        Self::install_with_release(literal, ReleaseOn::Explicit)
    }
    fn install_with_release(literal: impl Into<String>, release_on: ReleaseOn) -> Self {
        let literal = literal.into();
        let gate = Arc::new(Gate {
            state: Mutex::new(State::default()),
            changed: Condvar::new(),
            release_on,
        });
        assert!(
            registry()
                .lock()
                .unwrap()
                .insert(literal.clone(), gate.clone())
                .is_none()
        );
        Self { literal, gate }
    }
    pub fn wait_admitted(&self, count: usize, timeout: Duration) -> bool {
        let until = Instant::now() + timeout;
        let mut state = self.gate.state.lock().unwrap();
        while state.admitted < count {
            let remaining = until.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return false;
            }
            state = self.gate.changed.wait_timeout(state, remaining).unwrap().0;
        }
        true
    }
    pub fn entered(&self) -> usize {
        self.gate.state.lock().unwrap().entered
    }
    pub fn wait_entered(&self, count: usize, timeout: Duration) -> bool {
        let until = Instant::now() + timeout;
        let mut state = self.gate.state.lock().unwrap();
        while state.entered < count {
            let remaining = until.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return false;
            }
            state = self.gate.changed.wait_timeout(state, remaining).unwrap().0;
        }
        true
    }
    pub fn release(&self) {
        let mut state = self.gate.state.lock().unwrap();
        state.released = true;
        self.gate.changed.notify_all();
    }
    pub fn is_released(&self) -> bool {
        self.gate.state.lock().unwrap().released
    }
}
impl Drop for SearchBarrier {
    fn drop(&mut self) {
        self.release();
        registry().lock().unwrap().remove(&self.literal);
    }
}

pub(crate) fn pause(literal: &str, budget: &CallBudget, clock: &dyn Clock) {
    let gate = registry().lock().unwrap().get(literal).cloned();
    let Some(gate) = gate else { return };
    let mut state = gate.state.lock().unwrap();
    state.entered += 1;
    gate.changed.notify_all();
    while !state.released
        && match gate.release_on {
            ReleaseOn::Budget => !budget.is_exhausted(clock),
            ReleaseOn::Cancellation => !budget.cancellation.is_cancelled(),
            ReleaseOn::Explicit => true,
        }
    {
        state = gate
            .changed
            .wait_timeout(state, Duration::from_millis(5))
            .unwrap()
            .0;
    }
}

pub(crate) fn note_admitted(literal: &str) {
    let gate = registry().lock().unwrap().get(literal).cloned();
    if let Some(gate) = gate {
        let mut state = gate.state.lock().unwrap();
        state.admitted += 1;
        gate.changed.notify_all();
    }
}
