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
