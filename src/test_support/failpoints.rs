//! Test-only failpoints for crash-boundary tests.
//!
//! This module exists only under `cfg(test)` or the `test-support` feature.
//! Production code reaches it exclusively through the crate-private
//! `failpoint!` macro, which expands to nothing in ordinary builds. No wire
//! field, environment variable or settings file can arm a failpoint: arming
//! requires constructing [`Failpoint`] in Rust test code.
//!
//! Every failpoint is keyed by `(name, scope)`. Store hooks scope by database
//! path, service hooks by instance and scheduler hooks by daemon boot, so
//! concurrently running tests cannot trigger each other's failpoints.
use crate::protocol::results::{ApiError, ErrorCode};
use rusqlite::Connection;
use std::{
    collections::HashMap,
    sync::{Arc, Condvar, Mutex, OnceLock},
    time::{Duration, Instant},
};

type ConnectionAction = Box<dyn Fn(&Connection) + Send + Sync>;

enum Action {
    /// Return a typed error at the boundary. Inside a transaction this rolls
    /// back; after a commit it models a crash or lost response.
    Error(ErrorCode),
    /// Block the caller at the boundary until released.
    Pause,
    /// Run an action against the real connection (for example a PRAGMA that
    /// makes SQLite itself fail).
    Connection(ConnectionAction),
    /// Count every arrival and pass: a deterministic barrier that proves a
    /// caller reached the boundary (for example, is waiting for the writer).
    Observe,
}

#[derive(Default)]
struct State {
    hits: usize,
    fired: usize,
    released: bool,
}

struct Gate {
    action: Action,
    times: usize,
    state: Mutex<State>,
    changed: Condvar,
}

type Key = (&'static str, String);

fn registry() -> &'static Mutex<HashMap<Key, Arc<Gate>>> {
    static REGISTRY: OnceLock<Mutex<HashMap<Key, Arc<Gate>>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

/// An armed failpoint. Dropping it releases any paused caller and disarms it.
pub struct Failpoint {
    key: Key,
    gate: Arc<Gate>,
}

impl Failpoint {
    fn install(name: &'static str, scope: impl Into<String>, action: Action, times: usize) -> Self {
        let key = (name, scope.into());
        let gate = Arc::new(Gate {
            action,
            times,
            state: Mutex::new(State::default()),
            changed: Condvar::new(),
        });
        let prior = registry().lock().unwrap().insert(key.clone(), gate.clone());
        assert!(prior.is_none(), "failpoint {key:?} armed twice");
        Self { key, gate }
    }

    /// Fail the next hit with a typed error.
    pub fn error(name: &'static str, scope: impl Into<String>, code: ErrorCode) -> Self {
        Self::install(name, scope, Action::Error(code), 1)
    }

    /// Fail the next `times` hits with a typed error.
    pub fn error_times(
        name: &'static str,
        scope: impl Into<String>,
        code: ErrorCode,
        times: usize,
    ) -> Self {
        Self::install(name, scope, Action::Error(code), times)
    }

    /// Pause the next hit until [`Failpoint::release`] or drop.
    pub fn pause(name: &'static str, scope: impl Into<String>) -> Self {
        Self::install(name, scope, Action::Pause, 1)
    }

    /// Run `action` on the real connection at every hit while armed.
    pub fn connection(
        name: &'static str,
        scope: impl Into<String>,
        action: impl Fn(&Connection) + Send + Sync + 'static,
    ) -> Self {
        Self::install(
            name,
            scope,
            Action::Connection(Box::new(action)),
            usize::MAX,
        )
    }

    /// Count every arrival at the boundary and let it pass.
    pub fn observe(name: &'static str, scope: impl Into<String>) -> Self {
        Self::install(name, scope, Action::Observe, usize::MAX)
    }

    /// Every arrival at the boundary while armed, fired or not.
    pub fn hits(&self) -> usize {
        self.gate.state.lock().unwrap().hits
    }

    /// Arrivals at which the action ran.
    pub fn fired(&self) -> usize {
        self.gate.state.lock().unwrap().fired
    }

    /// Wait until `count` callers have fired this failpoint.
    pub fn wait_fired(&self, count: usize, timeout: Duration) -> bool {
        let until = Instant::now() + timeout;
        let mut state = self.gate.state.lock().unwrap();
        while state.fired < count {
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
}

impl Drop for Failpoint {
    fn drop(&mut self) {
        self.release();
        let mut registry = registry().lock().unwrap();
        if registry
            .get(&self.key)
            .is_some_and(|gate| Arc::ptr_eq(gate, &self.gate))
        {
            registry.remove(&self.key);
        }
    }
}

fn armed(name: &'static str, scope: &str) -> Option<Arc<Gate>> {
    let gate = registry()
        .lock()
        .unwrap()
        .get(&(name, scope.to_owned()))
        .cloned()?;
    let mut state = gate.state.lock().unwrap();
    state.hits += 1;
    if state.fired >= gate.times {
        return None;
    }
    state.fired += 1;
    gate.changed.notify_all();
    drop(state);
    Some(gate)
}

/// A boundary hook: returns the armed error, blocks while paused, or passes.
pub(crate) fn hit(name: &'static str, scope: &str) -> Result<(), ApiError> {
    run(name, scope, None)
}

/// A boundary hook that may also act on the real SQLite connection.
pub(crate) fn hit_connection(
    name: &'static str,
    scope: &str,
    conn: &Connection,
) -> Result<(), ApiError> {
    run(name, scope, Some(conn))
}

fn run(name: &'static str, scope: &str, conn: Option<&Connection>) -> Result<(), ApiError> {
    let Some(gate) = armed(name, scope) else {
        return Ok(());
    };
    match &gate.action {
        Action::Error(code) => Err(ApiError::new(
            code.clone(),
            format!("test failpoint {name}"),
        )),
        Action::Pause => {
            let mut state = gate.state.lock().unwrap();
            while !state.released {
                state = gate.changed.wait(state).unwrap();
            }
            Ok(())
        }
        Action::Connection(action) => {
            if let Some(conn) = conn {
                action(conn);
            }
            Ok(())
        }
        Action::Observe => Ok(()),
    }
}
