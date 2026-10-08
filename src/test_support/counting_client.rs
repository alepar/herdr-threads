//! Counting fake LocalClient (ht-p03.12.12) for B1 CLI read-cost tests.
//!
//! Wraps a real or scripted client, counts connections and calls per request
//! kind, and reports the hello capabilities of a chosen daemon vintage.

use crate::{
    ports::LocalClient,
    protocol::{
        capabilities::{ADVERTISED, Capabilities},
        commands::Command,
        output::OutputSpec,
        results::{ApiError, CapabilityList, CommandResult},
        time::CallBudget,
    },
};
use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU32, Ordering},
    },
};

/// The request kinds the B1 CLI leaves are costed by.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum CallKind {
    Seats,
    Participants,
    /// Goes through the host, not `LocalClient`; see [`CountingLocalClient::record_pane_names`].
    PaneNames,
    SeatInspect,
    History,
    Message,
    Capabilities,
    HookParseFailure,
    HarnessEvidence,
    Other,
}

impl CallKind {
    pub fn classify(command: &Command) -> Self {
        match command {
            Command::Seats(_) => Self::Seats,
            Command::Participants(_) => Self::Participants,
            Command::SeatInspect(_) => Self::SeatInspect,
            Command::History(_) => Self::History,
            Command::Message(_) => Self::Message,
            Command::Capabilities => Self::Capabilities,
            Command::HookParseFailure(_) => Self::HookParseFailure,
            Command::HarnessEvidence(_) => Self::HarnessEvidence,
            _ => Self::Other,
        }
    }
}

type Script = Box<dyn Fn(&Command) -> Result<CommandResult, ApiError> + Send + Sync>;

pub enum Backend {
    Real(Arc<dyn LocalClient>),
    Scripted(Script),
}

/// Which daemon the fake poses as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DaemonVintage {
    /// This build's daemon: advertises exactly `ADVERTISED`.
    Current,
    /// An older daemon: no capabilities.
    Older,
}

pub struct CountingLocalClient {
    backend: Backend,
    vintage: DaemonVintage,
    calls: Mutex<BTreeMap<CallKind, u32>>,
    connections: AtomicU32,
}

impl CountingLocalClient {
    pub fn real(inner: Arc<dyn LocalClient>) -> Self {
        Self::new(Backend::Real(inner), DaemonVintage::Current)
    }

    pub fn scripted(
        reply: impl Fn(&Command) -> Result<CommandResult, ApiError> + Send + Sync + 'static,
        vintage: DaemonVintage,
    ) -> Self {
        Self::new(Backend::Scripted(Box::new(reply)), vintage)
    }

    fn new(backend: Backend, vintage: DaemonVintage) -> Self {
        Self {
            backend,
            vintage,
            calls: Mutex::new(BTreeMap::new()),
            connections: AtomicU32::new(0),
        }
    }

    /// Counts one connection; the CLI under test calls this where production
    /// code constructs a client.
    pub fn connect(&self) -> &Self {
        self.connections.fetch_add(1, Ordering::SeqCst);
        self
    }

    pub fn connections(&self) -> u32 {
        self.connections.load(Ordering::SeqCst)
    }

    pub fn calls(&self, kind: CallKind) -> u32 {
        self.counts().get(&kind).copied().unwrap_or(0)
    }

    pub fn total_calls(&self) -> u32 {
        self.counts().values().sum()
    }

    /// `pane_names` goes through the host, not `LocalClient`: the CLI test host
    /// calls this once per lookup.
    pub fn record_pane_names(&self) {
        self.count(CallKind::PaneNames);
    }

    /// The capabilities the session reports, per vintage (the same type as
    /// `LocalSocketClient::capabilities`).
    pub fn capabilities(&self) -> Capabilities {
        match self.vintage {
            DaemonVintage::Older => Capabilities::none(),
            DaemonVintage::Current => {
                Capabilities::from_list(ADVERTISED.iter().map(|name| (*name).to_string()))
            }
        }
    }

    fn counts(&self) -> std::sync::MutexGuard<'_, BTreeMap<CallKind, u32>> {
        self.calls
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn count(&self, kind: CallKind) {
        *self.counts().entry(kind).or_insert(0) += 1;
    }

    /// Counts the command, then serves it. A scripted backend answers
    /// `Command::Capabilities` from its vintage so the wire request and the
    /// accessor agree; the script sees every other command.
    fn serve(
        &self,
        command: Command,
        real: impl FnOnce(&Arc<dyn LocalClient>, Command) -> Result<CommandResult, ApiError>,
    ) -> Result<CommandResult, ApiError> {
        self.count(CallKind::classify(&command));
        match &self.backend {
            Backend::Real(inner) => real(inner, command),
            Backend::Scripted(_) if command == Command::Capabilities => {
                Ok(CommandResult::Capabilities(CapabilityList {
                    capabilities: match self.vintage {
                        DaemonVintage::Older => Vec::new(),
                        DaemonVintage::Current => {
                            ADVERTISED.iter().map(|name| (*name).to_string()).collect()
                        }
                    },
                }))
            }
            Backend::Scripted(script) => script(&command),
        }
    }
}

impl LocalClient for CountingLocalClient {
    fn supports_capability(&self, name: &str, _budget: &CallBudget) -> bool {
        self.capabilities().supports(name)
    }
    fn call(&self, command: Command, budget: &CallBudget) -> Result<CommandResult, ApiError> {
        self.serve(command, |inner, command| inner.call(command, budget))
    }

    fn call_with_output(
        &self,
        command: Command,
        output: &OutputSpec,
        budget: &CallBudget,
    ) -> Result<CommandResult, ApiError> {
        self.serve(command, |inner, command| {
            inner.call_with_output(command, output, budget)
        })
    }

    fn call_definitive(
        &self,
        command: Command,
        budget: &CallBudget,
    ) -> Result<Result<CommandResult, ApiError>, ApiError> {
        match &self.backend {
            Backend::Real(inner) => {
                self.count(CallKind::classify(&command));
                inner.call_definitive(command, budget)
            }
            Backend::Scripted(_) => self.serve(command, |_, _| unreachable!()).map(Ok),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{
        capabilities::HISTORY_FULL_BODIES,
        commands::{BodyReadRequest, HistoryQuery, MessageQuery},
        ids::{MessageId, ThreadId},
        pagination::PageRequest,
        time::{Cancellation, MonoInstant},
    };

    fn budget() -> CallBudget {
        CallBudget {
            deadline: MonoInstant(1_000),
            cancellation: Cancellation::default(),
        }
    }

    fn ok_script(_: &Command) -> Result<CommandResult, ApiError> {
        Ok(CommandResult::Capabilities(CapabilityList {
            capabilities: vec![],
        }))
    }

    fn history() -> Command {
        Command::History(HistoryQuery {
            thread: ThreadId::new("t1"),
            page: PageRequest::default(),
            initial: None,
            full_bodies: false,
        })
    }

    fn message() -> Command {
        Command::Message(MessageQuery {
            message: MessageId::new("m1"),
            body: BodyReadRequest {
                cursor: None,
                offset: None,
                max_bytes: 1024,
            },
        })
    }

    #[test]
    fn two_calls_are_counted_per_kind() {
        let fake = CountingLocalClient::scripted(ok_script, DaemonVintage::Current);
        fake.call(history(), &budget()).unwrap();
        fake.call(message(), &budget()).unwrap();
        assert_eq!(fake.calls(CallKind::History), 1);
        assert_eq!(fake.calls(CallKind::Message), 1);
        assert_eq!(fake.calls(CallKind::Seats), 0);
        assert_eq!(fake.total_calls(), 2);
    }

    #[test]
    fn older_daemon_reports_no_full_bodies_capability() {
        let fake = CountingLocalClient::scripted(ok_script, DaemonVintage::Older);
        assert!(!fake.capabilities().supports(HISTORY_FULL_BODIES));
        assert!(fake.capabilities().is_empty());
        let CommandResult::Capabilities(list) =
            fake.call(Command::Capabilities, &budget()).unwrap()
        else {
            panic!("capabilities result");
        };
        assert!(list.capabilities.is_empty());
    }

    #[test]
    fn current_daemon_reports_the_advertised_set() {
        let fake = CountingLocalClient::scripted(ok_script, DaemonVintage::Current);
        assert_eq!(
            fake.capabilities(),
            Capabilities::from_list(ADVERTISED.iter().map(|n| n.to_string()))
        );
        let CommandResult::Capabilities(list) =
            fake.call(Command::Capabilities, &budget()).unwrap()
        else {
            panic!("capabilities result");
        };
        assert_eq!(list.capabilities, ADVERTISED);
        assert_eq!(fake.calls(CallKind::Capabilities), 1);
    }

    #[test]
    fn connections_are_counted() {
        let fake = CountingLocalClient::scripted(ok_script, DaemonVintage::Current);
        assert_eq!(fake.connections(), 0);
        fake.connect().connect();
        assert_eq!(fake.connections(), 2);
        assert_eq!(fake.total_calls(), 0);
        fake.record_pane_names();
        assert_eq!(fake.calls(CallKind::PaneNames), 1);
    }

    #[test]
    fn real_backend_delegates_and_counts_all_call_forms() {
        let inner = Arc::new(CountingLocalClient::scripted(
            ok_script,
            DaemonVintage::Current,
        ));
        let fake = CountingLocalClient::real(inner.clone());
        fake.call(history(), &budget()).unwrap();
        fake.call_definitive(message(), &budget()).unwrap().unwrap();
        assert_eq!(fake.total_calls(), 2);
        assert_eq!(inner.calls(CallKind::History), 1);
        assert_eq!(inner.calls(CallKind::Message), 1);
    }
}
