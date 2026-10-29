use crate::{
    cli::run_wire,
    ports::LocalClient,
    protocol::{
        commands::{Command, ServiceDisconnectRequest},
        output::{ContinuationContext, OutputFormat, OutputSpec},
        results::{
            ApiError, CommandResult, ServiceConnectionInspection, ServiceDisconnectResult,
            ServiceRecoveryAudit,
        },
        time::{CallBudget, MonoInstant, UtcMillis},
    },
};
use std::sync::Mutex;

const INSTANCE: &str = "00000000-0000-4000-8000-000000000001";
const BOOT: &str = "00000000-0000-4000-8000-000000000002";

#[derive(Default)]
struct RecoveryClient(Mutex<Vec<&'static str>>);

impl LocalClient for RecoveryClient {
    fn call(&self, command: Command, _: &CallBudget) -> Result<CommandResult, ApiError> {
        self.0.lock().unwrap().push("default");
        Ok(match command {
            Command::ServiceInspect => {
                CommandResult::ServiceInspection(ServiceConnectionInspection {
                    instance: INSTANCE.into(),
                    daemon_boot: BOOT.into(),
                    connected: true,
                    connection_generation: Some(7),
                    registered_at: Some(UtcMillis(0)),
                })
            }
            Command::ServiceDisconnect(request) => {
                assert_eq!(request.expected_boot, BOOT);
                assert_eq!(request.expected_generation, 7);
                CommandResult::ServiceDisconnected(ServiceDisconnectResult {
                    instance: INSTANCE.into(),
                    daemon_boot: BOOT.into(),
                    connection_generation: 7,
                    disconnected: true,
                    audit: ServiceRecoveryAudit::Persisted,
                })
            }
            other => panic!("unexpected command: {other:?}"),
        })
    }

    fn call_with_output(
        &self,
        _: Command,
        _: &OutputSpec,
        _: &CallBudget,
    ) -> Result<CommandResult, ApiError> {
        self.0.lock().unwrap().push("selected");
        Err(ApiError::unsupported(
            "selected output is unavailable for service recovery",
        ))
    }
}

#[test]
fn service_disconnect_uses_default_wire_request_and_selected_local_output() {
    let client = RecoveryClient::default();
    let output = OutputSpec {
        format: OutputFormat::Text,
        context: ContinuationContext::default(),
    };
    let mut bytes = Vec::new();
    run_wire(
        Command::ServiceDisconnect(ServiceDisconnectRequest {
            expected_boot: BOOT.into(),
            expected_generation: 7,
        }),
        &output,
        &client,
        &|| CallBudget {
            deadline: MonoInstant(u64::MAX),
            cancellation: Default::default(),
        },
        &mut bytes,
    )
    .unwrap();
    let text = String::from_utf8(bytes).unwrap();
    assert!(text.contains("service_disconnected"));
    assert!(text.contains("connection_generation: 7"));
    assert_eq!(*client.0.lock().unwrap(), ["default"]);
}

#[test]
fn service_inspect_uses_default_wire_request_and_selected_local_output() {
    let client = RecoveryClient::default();
    for (format, expected) in [
        (OutputFormat::Text, "service_inspection\n"),
        (OutputFormat::Json, "\"kind\":\"service_inspection\""),
    ] {
        let output = OutputSpec {
            format,
            context: ContinuationContext {
                state_dir: Some("/tmp/service-state".into()),
                host: None,
            },
        };
        let mut bytes = Vec::new();
        run_wire(
            Command::ServiceInspect,
            &output,
            &client,
            &|| CallBudget {
                deadline: MonoInstant(u64::MAX),
                cancellation: Default::default(),
            },
            &mut bytes,
        )
        .unwrap();
        assert!(String::from_utf8(bytes).unwrap().contains(expected));
    }
    assert_eq!(*client.0.lock().unwrap(), ["default", "default"]);
}

/// A daemon from before 2.8.0: no capabilities, and no health answer either.
#[derive(Default)]
struct OlderDaemon(Mutex<Vec<String>>);

impl LocalClient for OlderDaemon {
    fn call(&self, command: Command, _: &CallBudget) -> Result<CommandResult, ApiError> {
        self.0.lock().unwrap().push(format!("{command:?}"));
        Err(ApiError::unsupported("older daemon"))
    }

    fn call_with_output(
        &self,
        command: Command,
        _: &OutputSpec,
        _: &CallBudget,
    ) -> Result<CommandResult, ApiError> {
        panic!("{command:?} reached a daemon without its capability")
    }
}

// Kills: a new peer-hygiene command sent to an older daemon (a raw
// unknown-variant decode error mid-upgrade instead of one clear line).
#[test]
fn peer_hygiene_commands_refuse_one_line_on_an_older_daemon() {
    use crate::protocol::sync_admin::SyncCommand;
    for (sync, what) in [
        (
            SyncCommand::PeerPrune {
                host: None,
                older_than_ms: 1,
                apply: false,
                list: false,
            },
            "peer prune/unforget",
        ),
        (
            SyncCommand::PeerUnforget {
                target: "Abc".into(),
                apply: true,
            },
            "peer prune/unforget",
        ),
        (
            SyncCommand::PeerEvents {
                node: None,
                before: None,
                limit: 1,
            },
            "peer events",
        ),
    ] {
        let client = OlderDaemon::default();
        let mut bytes = Vec::new();
        let error = run_wire(
            Command::Sync(sync),
            &OutputSpec {
                format: OutputFormat::Text,
                context: ContinuationContext::default(),
            },
            &client,
            &|| CallBudget {
                deadline: MonoInstant(u64::MAX),
                cancellation: Default::default(),
            },
            &mut bytes,
        )
        .unwrap_err();
        let text = format!("{error:?}");
        assert!(
            text.contains(&format!(
                "the running daemon (an older version) does not support {what}; after upgrading run: herdr-threads daemon stop && herdr-threads daemon ensure"
            )),
            "{text}"
        );
        assert!(bytes.is_empty());
        assert_eq!(client.0.lock().unwrap().len(), 1, "one health read only");
    }
}
