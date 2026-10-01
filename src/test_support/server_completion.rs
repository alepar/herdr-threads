//! Feature-gated observations tied to the server's worker and permit lifetimes.
use crate::protocol::commands::Command;
use std::{
    collections::HashMap,
    sync::{Arc, Condvar, Mutex, OnceLock},
    time::{Duration, Instant},
};

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
enum Key {
    Search(String),
    Resolution(String),
}
impl Key {
    fn command(command: &Command) -> Option<Self> {
        match command {
            Command::Search(query) => Some(Self::Search(query.literal.clone())),
            Command::ResolveSeat(request) => {
                Some(Self::Resolution(request.operation.as_str().to_owned()))
            }
            _ => None,
        }
    }
}

#[derive(Default)]
struct State {
    entered: usize,
    exited: usize,
    search_permits_returned: usize,
}
#[derive(Default)]
struct Signal {
    state: Mutex<State>,
    changed: Condvar,
}
fn registry() -> &'static Mutex<HashMap<Key, Arc<Signal>>> {
    static REGISTRY: OnceLock<Mutex<HashMap<Key, Arc<Signal>>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}
fn observed(command: &Command) -> Option<Arc<Signal>> {
    let key = Key::command(command)?;
    registry().lock().unwrap().get(&key).cloned()
}

/// Install before a unique semantic request is submitted over the real socket.
pub struct ServerCompletion {
    key: Key,
    signal: Arc<Signal>,
}
impl ServerCompletion {
    fn install(key: Key) -> Self {
        let signal = Arc::new(Signal::default());
        assert!(
            registry()
                .lock()
                .unwrap()
                .insert(key.clone(), signal.clone())
                .is_none()
        );
        Self { key, signal }
    }
    pub fn search(literal: impl Into<String>) -> Self {
        Self::install(Key::Search(literal.into()))
    }
    pub fn resolution(operation: impl Into<String>) -> Self {
        Self::install(Key::Resolution(operation.into()))
    }
    fn wait(&self, timeout: Duration, count: impl Fn(&State) -> usize) -> bool {
        let until = Instant::now() + timeout;
        let mut state = self.signal.state.lock().unwrap();
        while count(&state) == 0 {
            let remaining = until.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return false;
            }
            state = self
                .signal
                .changed
                .wait_timeout(state, remaining)
                .unwrap()
                .0;
        }
        true
    }
    pub fn wait_worker_entered(&self, timeout: Duration) -> bool {
        self.wait(timeout, |state| state.entered)
    }
    pub fn wait_worker_exited(&self, timeout: Duration) -> bool {
        self.wait(timeout, |state| state.exited)
    }
    pub fn wait_search_permits_returned(&self, timeout: Duration) -> bool {
        self.wait(timeout, |state| state.search_permits_returned)
    }
}
impl Drop for ServerCompletion {
    fn drop(&mut self) {
        registry().lock().unwrap().remove(&self.key);
    }
}

/// Its destructor runs inside the blocking closure, after the handler returns.
pub(crate) struct WorkerExit(Option<Arc<Signal>>);
impl WorkerExit {
    pub(crate) fn enter(command: &Command) -> Self {
        let signal = observed(command);
        if let Some(signal) = &signal {
            let mut state = signal.state.lock().unwrap();
            state.entered += 1;
            signal.changed.notify_all();
        }
        Self(signal)
    }
}
impl Drop for WorkerExit {
    fn drop(&mut self) {
        if let Some(signal) = &self.0 {
            let mut state = signal.state.lock().unwrap();
            state.exited += 1;
            signal.changed.notify_all();
        }
    }
}

/// Declare before both search permit locals. Its destructor then runs only
/// after both permit destructors have returned their semaphore capacity.
pub(crate) struct SearchPermitsReturned {
    signal: Option<Arc<Signal>>,
    armed: bool,
}
impl SearchPermitsReturned {
    pub(crate) fn new(command: &Command) -> Self {
        let signal = matches!(command, Command::Search(_))
            .then(|| observed(command))
            .flatten();
        Self {
            signal,
            armed: false,
        }
    }
    pub(crate) fn arm(&mut self, admitted: bool) {
        self.armed = admitted;
    }
}
impl Drop for SearchPermitsReturned {
    fn drop(&mut self) {
        if self.armed
            && let Some(signal) = &self.signal
        {
            let mut state = signal.state.lock().unwrap();
            state.search_permits_returned += 1;
            signal.changed.notify_all();
        }
    }
}
